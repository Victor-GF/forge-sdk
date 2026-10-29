//! Grafo "frame": roda uma vez por frame real e só lê o RenderSnapshot (mais o cenário
//! estático). Simples onde pode (cenário assado), caro onde importa (carro "hero").

use std::f32::consts::TAU;

use forja::draw::{add, channels, from_f, mix, scale, Canvas};
use forja::math::{lerp, lerp_angle, v2, Rng, V2};
use forja::schedule::{SysCtx, SysFn};

use crate::tables::*;
use crate::track::StaticScene;

pub const DT: f32 = 1.0 / 60.0;

/// Sistemas do grafo de render, pelo nome (a declaração fica em `projeto.forja`).
pub fn register(fns: &mut Vec<(&'static str, SysFn)>) {
    fns.extend_from_slice(&[("Render.Scene", render_scene as SysFn), ("Render.Cars", render_cars), ("Render.VFX", render_vfx)]);
}

#[derive(Clone, Copy)]
pub struct View {
    pub pos: V2,
    pub zoom: f32,
    pub angle: f32,
    cx: f32,
    cy: f32,
    cos: f32,
    sin: f32,
}

impl View {
    pub fn new(cam: CamSnap, w: usize, h: usize) -> View {
        let (sin, cos) = cam.angle.sin_cos();
        View { pos: cam.pos, zoom: cam.zoom.max(0.5), angle: cam.angle, cx: w as f32 * 0.5, cy: h as f32 * 0.5, cos, sin }
    }

    pub fn interp(snap: &RenderSnapshot, alpha: f32, w: usize, h: usize) -> View {
        let a = snap.cam_prev;
        let b = snap.cam;
        View::new(
            CamSnap {
                pos: a.pos.lerp(b.pos, alpha),
                zoom: lerp(a.zoom, b.zoom, alpha),
                angle: lerp_angle(a.angle, b.angle, alpha),
                ..b
            },
            w,
            h,
        )
    }

    #[inline]
    pub fn to_screen(&self, p: V2) -> V2 {
        let d = p - self.pos;
        v2(d.x * self.cos + d.y * self.sin, -d.x * self.sin + d.y * self.cos) * self.zoom + v2(self.cx, self.cy)
    }

    #[inline]
    pub fn to_world(&self, s: V2) -> V2 {
        let d = (s - v2(self.cx, self.cy)) / self.zoom;
        v2(d.x * self.cos - d.y * self.sin, d.x * self.sin + d.y * self.cos) + self.pos
    }

    /// Direção no mundo -> direção na tela.
    pub fn dir(&self, a: f32) -> V2 {
        V2::from_angle(a - self.angle)
    }
}

fn band_visible(c: &Canvas, s: V2, r: f32) -> bool {
    s.y + r >= c.y0 as f32 && s.y - r < c.y1 as f32 && s.x + r >= 0.0 && s.x - r < c.w as f32
}

// ------------------------------------------------------------------ cenário

fn render_scene(ctx: &mut SysCtx) {
    let snap = ctx.read::<RenderSnapshot>();
    let scene = ctx.read::<StaticScene>();
    let mut fbw = ctx.write::<Framebuffer>();
    let fb = &mut *fbw;
    let (w, h) = (fb.w, fb.h);
    if !snap.valid {
        fb.px.fill(0);
        return;
    }
    let view = View::interp(&snap, ctx.alpha(), w, h);
    let snap: &RenderSnapshot = &snap;
    let scene: &StaticScene = &scene;
    ctx.par_rows(&mut fb.px, w, h, |c| {
        for y in c.y0..c.y1 {
            let p0 = view.to_world(v2(0.5, y as f32 + 0.5));
            let dx = view.to_world(v2(1.5, y as f32 + 0.5)) - p0;
            let mut p = p0;
            let row = (y - c.y0) * w;
            for px in &mut c.px[row..row + w] {
                *px = scene.sample(p);
                p += dx;
            }
        }
        for o in &snap.obstacles {
            draw_obstacle(c, &view, o);
        }
    });
}

fn draw_obstacle(c: &mut Canvas, view: &View, o: &ObsSnap) {
    let s = view.to_screen(o.pos);
    let r = o.radius * view.zoom;
    if !band_visible(c, s, r + 6.0) {
        return;
    }
    let sh = v2(0.5, 0.6) * view.zoom;
    match o.kind {
        OBS_CONE => {
            c.circle(s + sh * 0.6, r, 0, 0.35);
            c.circle(s, r, o.color, 1.0);
            c.ring(s, r * 0.55, (r * 0.22).max(1.0), 0xF5F5F5, 1.0);
            c.circle(s - v2(r, r) * 0.2, r * 0.25, 0xFFD2A8, 0.8);
        }
        OBS_BARREL => {
            c.circle(s + sh, r, 0, 0.35);
            c.circle(s, r, o.color, 1.0);
            c.ring(s, r * 0.7, (r * 0.12).max(1.0), scale(o.color, 0.6), 1.0);
            c.circle(s - v2(r, r) * 0.3, r * 0.3, 0xFFFFFF, 0.35);
        }
        _ => {
            let hu = view.dir(o.angle) * r;
            let hv = hu.perp();
            c.quad_shader(s + sh, hu, hv, |_, _, _, _| Some((0, 0.35)));
            c.quad_shader(s, hu, hv, |u, v, _, _| {
                let edge = u.abs().max(v.abs()) > 0.8;
                let stripe = (((u + v) * 2.5).floor() as i32).rem_euclid(2) == 0;
                let col = if edge {
                    if stripe {
                        0xFACC15
                    } else {
                        0x1F1F1F
                    }
                } else {
                    mix(o.color, 0xFFFFFF, (0.3 - v * 0.2).clamp(0.0, 0.4))
                };
                Some((col, 1.0))
            });
        }
    }
}

// ------------------------------------------------------------------ carros (pass hero)

fn render_cars(ctx: &mut SysCtx) {
    let snap = ctx.read::<RenderSnapshot>();
    let scene = ctx.read::<StaticScene>();
    let mut fbw = ctx.write::<Framebuffer>();
    let fb = &mut *fbw;
    let (w, h) = (fb.w, fb.h);
    if !snap.valid {
        return;
    }
    let alpha = ctx.alpha();
    let view = View::interp(&snap, alpha, w, h);
    let flicker = (ctx.tick() / 4) % 2 == 0;
    let snap: &RenderSnapshot = &snap;
    let scene: &StaticScene = &scene;
    ctx.par_rows(&mut fb.px, w, h, |c| {
        for i in 0..snap.cars.len() {
            let a = snap.cars_prev.get(i).copied().unwrap_or(snap.cars[i]);
            let b = snap.cars[i];
            let teleported = (b.pos - a.pos).len() > 20.0;
            let pos = if teleported { b.pos } else { a.pos.lerp(b.pos, alpha) };
            let ang = if teleported { b.angle } else { lerp_angle(a.angle, b.angle, alpha) };
            draw_car(c, &view, scene, pos, ang, &b, flicker);
        }
    });
}

const SKY: u32 = 0xBFD9F2;

fn draw_car(c: &mut Canvas, view: &View, scene: &StaticScene, pos: V2, ang: f32, car: &CarSnap, flicker: bool) {
    let center = view.to_screen(pos);
    let reach = 2.6 * view.zoom + 4.0;
    if !band_visible(c, center, reach) {
        return;
    }
    let f = view.dir(ang);
    let hu = f * (2.3 * view.zoom);
    let hv = f.perp() * (1.0 * view.zoom);
    let shape = |u: f32, v: f32| u.abs().powi(6) + v.abs().powi(6);

    // sombra (sol vindo de cima-esquerda, como no cenário assado)
    c.quad_shader(center + v2(0.6, 0.8) * view.zoom, hu * 1.03, hv * 1.1, |u, v, _, _| {
        let d = shape(u, v);
        if d <= 1.0 {
            Some((0, 0.38 * ((1.0 - d) / 0.3).min(1.0)))
        } else {
            None
        }
    });

    let alpha_body = if car.ghost && flicker { 0.45 } else { 1.0 };
    let l = {
        let v = (-0.45f32, -0.6f32, 0.66f32);
        let n = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
        (v.0 / n, v.1 / n, v.2 / n)
    };
    let hl = {
        let v = (l.0, l.1, l.2 + 1.0);
        let n = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
        (v.0 / n, v.1 / n, v.2 / n)
    };
    let (sa, ca) = ang.sin_cos();
    c.quad_shader(center, hu, hv, |u, v, x, y| {
        let d = shape(u, v);
        if d > 1.0 {
            return None;
        }
        // normal local: carroceria abaulada na largura, caída na frente/traseira
        let nv = v * 0.75;
        let nu = u.powi(5) * 0.6;
        let nz = (1.0 - nv * nv - nu * nu).max(0.05).sqrt();
        let nx = nu * ca - nv * sa;
        let ny = nu * sa + nv * ca;
        let ndl = (nx * l.0 + ny * l.1 + nz * l.2).max(0.0);
        let ndh = (nx * hl.0 + ny * hl.1 + nz * hl.2).max(0.0);
        let spec = ndh.powi(48) + ndh.powi(6) * 0.12;
        let fres = 0.04 + 0.96 * (1.0 - nz).powi(5);
        // reflexo: o verniz espelha o que está ao redor (cenário assado) e o céu
        let world = view.to_world(v2(x as f32 + 0.5, y as f32 + 0.5));
        let r_xy = v2(2.0 * nz * nx, 2.0 * nz * ny);
        let r_z = 2.0 * nz * nz - 1.0;
        let env = mix(scene.sample(world + r_xy * 7.0), SKY, r_z.clamp(0.0, 1.0));

        let glass = (u > 0.08 && u < 0.42 && v.abs() < 0.74) || (u < -0.45 && u > -0.7 && v.abs() < 0.7);
        let roof = (-0.45..=0.08).contains(&u) && v.abs() < 0.66;
        let mut base = car.color;
        if roof {
            base = scale(base, 0.9);
        }
        if glass {
            base = 0x1B2433;
        }
        if car.crashing {
            let n = (forja::math::hash2(x as i32 / 2, y as i32 / 2, 5) & 63) as f32 / 63.0;
            base = scale(base, 0.45 + n * 0.25);
        }
        let refl = if glass { 0.55 + 0.4 * fres } else { 0.12 + 0.6 * fres };
        let mut col = scale(base, 0.28 + 0.72 * ndl);
        col = mix(col, env, refl);
        col = add(col, 0xFFFFFF, spec * if glass { 1.4 } else { 1.1 });
        let av = v.abs();
        if u > 0.9 && av > 0.45 && av < 0.85 {
            col = 0xFFF6C8;
        }
        if u < -0.93 && av > 0.4 && av < 0.85 {
            col = if car.brake > 0.1 { 0xFF3030 } else { 0x8A1010 };
        }
        if d > 0.8 {
            col = scale(col, 0.55);
        }
        Some((col, alpha_body))
    });
    if car.boost {
        let tail = center - f * (2.6 * view.zoom);
        c.circle(tail, 0.55 * view.zoom, 0x60A5FA, 0.6);
        c.circle(tail - f * (0.5 * view.zoom), 0.35 * view.zoom, 0xFFFFFF, 0.8);
    }
}

// ------------------------------------------------------------------ VFX e pós

fn render_vfx(ctx: &mut SysCtx) {
    let snap = ctx.read::<RenderSnapshot>();
    let mut fbw = ctx.write::<Framebuffer>();
    let fb = &mut *fbw;
    let (w, h) = (fb.w, fb.h);
    if !snap.valid {
        return;
    }
    let alpha = ctx.alpha();
    let view = View::interp(&snap, alpha, w, h);
    let cam = snap.cam;
    let cine = lerp(snap.cam_prev.cine, cam.cine, alpha);
    let flash = cam.flash;
    let speedlines = cam.speedlines;
    let tick = ctx.tick();
    let snap: &RenderSnapshot = &snap;
    ctx.par_rows(&mut fb.px, w, h, |c| {
        for p in &snap.parts {
            let pos = p.pos + p.vel * (alpha * DT);
            let s = view.to_screen(pos);
            let r = (p.size * view.zoom).max(0.8);
            if !band_visible(c, s, r * 1.6 + 20.0) {
                continue;
            }
            match p.kind {
                P_SPARK => {
                    let tail = view.to_screen(pos - p.vel * 0.035);
                    c.line_add(tail, s, p.color, p.t.min(1.0));
                    c.circle(s, r.max(1.0), 0xFFFFFF, p.t);
                }
                P_SMOKE | P_DUST => c.circle(s, r, p.color, 0.32 * p.t),
                P_DEBRIS => {
                    let hu = V2::from_angle(p.rot - view.angle) * r;
                    let col = p.color;
                    c.quad_shader(s, hu, hu.perp() * 0.6, |u, _, _, _| Some((scale(col, 0.75 + 0.25 * u), 1.0)));
                }
                P_INK => draw_ink(c, s, r, p),
                P_RING => {
                    c.ring(s, r, (0.35 * view.zoom).max(2.0), 0xFFFFFF, p.t * 0.9);
                    c.ring(s, r * 0.8, (0.2 * view.zoom).max(1.0), 0x22E0FF, p.t * 0.7);
                }
                P_CONFETTI => {
                    let hu = V2::from_angle(p.rot) * r;
                    c.quad_shader(s, hu, hu.perp(), |_, _, _, _| Some((p.color, p.t.min(1.0))));
                }
                _ => {}
            }
        }

        // linhas de velocidade em espaço de tela
        if speedlines > 0.05 {
            let mut rng = Rng::new(tick.wrapping_mul(13) + 5);
            let center = v2(w as f32 * 0.5, h as f32 * 0.5);
            for _ in 0..36 {
                let a = rng.range(0.0, TAU);
                let r0 = rng.range(0.42, 0.62) * w as f32;
                let len = rng.range(40.0, 140.0) * speedlines;
                let d = V2::from_angle(a);
                c.line(center + d * r0, center + d * (r0 + len), 0xFFFFFF, 0.35 * speedlines);
            }
        }

        // grade cinematográfica do crash: dessatura, vinheta com retícula e letterbox.
        // Só custa nos frames de crash: aparece como outro caminho crítico na timeline.
        if cine > 0.02 {
            let cx = w as f32 * 0.5;
            let cy = h as f32 * 0.5;
            for y in c.y0..c.y1 {
                let dy = (y as f32 - cy) / cy;
                for x in 0..w {
                    let dx = (x as f32 - cx) / cx;
                    let vig = ((dx * dx + dy * dy) * 0.6).min(1.0) * cine;
                    let (r, g, b) = channels(c.get(x, y));
                    let lum = 0.3 * r + 0.59 * g + 0.11 * b;
                    let ds = 0.35 * cine;
                    let mut dark = 1.0 - vig * 0.55;
                    if vig > 0.3 {
                        let fx = (x % 6) as f32 - 2.5;
                        let fy = (y % 6) as f32 - 2.5;
                        if fx * fx + fy * fy < vig * 9.0 {
                            dark *= 0.6;
                        }
                    }
                    let col = from_f(lerp(r, lum, ds) * dark * 1.04, lerp(g, lum, ds) * dark, lerp(b, lum, ds) * dark * 0.96);
                    c.set(x, y, col);
                }
            }
            let bar = (h as f32 * 0.085 * cine) as i32;
            c.fill_rect(0, 0, w as i32, bar, 0, 1.0);
            c.fill_rect(0, h as i32 - bar, w as i32, h as i32, 0, 1.0);
        }
        if flash > 0.02 {
            c.fill_rect(0, c.y0 as i32, w as i32, c.y1 as i32, 0xFFFFFF, flash * 0.6);
        }
    });
}

/// "Adesivo" estilizado: forma chapada com contorno preto e pop de escala.
fn draw_ink(c: &mut Canvas, s: V2, r: f32, p: &PartSnap) {
    let age = 1.0 - p.t;
    let pop = if age < 0.12 { 0.4 + age / 0.12 * 0.9 } else { 1.0 + (1.0 - (age - 0.12) * 4.0).max(0.0) * 0.3 };
    let fade = (p.t / 0.25).min(1.0);
    let rr = r * pop * 1.1;
    let shape = (p.color >> 3) % 3;
    let pts: Vec<V2> = match shape {
        0 => (0..3).map(|k| s + V2::from_angle(p.rot + k as f32 * TAU / 3.0) * rr).collect(),
        1 => (0..4).map(|k| s + V2::from_angle(p.rot + k as f32 * TAU / 4.0) * (rr * if k % 2 == 0 { 1.0 } else { 0.55 })).collect(),
        _ => (0..4).map(|k| s + V2::from_angle(p.rot + k as f32 * TAU / 4.0 + 0.785) * (rr * 0.75)).collect(),
    };
    let outline: Vec<V2> = pts.iter().map(|q| s + (*q - s) * 1.3).collect();
    c.poly(&outline, 0x0A0A0A, fade);
    c.poly(&pts, p.color, fade);
}
