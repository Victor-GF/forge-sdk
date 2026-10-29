//! Cook da pista: spline -> amostras (tabela Track) e cenário assado numa imagem
//! (tabela StaticScene). Tudo isto é estático em runtime: lido em paralelo, sem custo
//! de sincronização, e a iluminação já vem pronta.

use forja::draw::{mix, rgb, scale, Canvas};
use forja::jobs::JobPool;
use forja::math::{hash2, value_noise, v2, wrap_angle, Rng, V2};
use forja::world::Table;

use crate::tables::{Obstacle, CARS, OBS_BARREL, OBS_BLOCK, OBS_CONE};

pub const HALF_WIDTH: f32 = 9.0;
pub const WALL: f32 = 23.0;
const STEP: f32 = 2.0;

const CONTROL: [(f32, f32); 17] = [
    (200.0, 200.0),
    (600.0, 160.0),
    (950.0, 200.0),
    (1080.0, 330.0),
    (1050.0, 480.0),
    (900.0, 520.0),
    (780.0, 620.0),
    (820.0, 780.0),
    (980.0, 850.0),
    (1050.0, 1000.0),
    (900.0, 1080.0),
    (600.0, 1060.0),
    (420.0, 960.0),
    (330.0, 800.0),
    (180.0, 720.0),
    (130.0, 520.0),
    (150.0, 330.0),
];

#[derive(Clone)]
pub struct Track {
    pub pos: Vec<V2>,
    pub tan: Vec<V2>,
    pub s: Vec<f32>,
    pub curv: Vec<f32>,
    pub length: f32,
}

impl Table for Track {
    fn rows(&self) -> usize {
        self.pos.len()
    }
}

fn catmull(p0: V2, p1: V2, p2: V2, p3: V2, t: f32) -> V2 {
    let t2 = t * t;
    let t3 = t2 * t;
    (p1 * 2.0 + (p2 - p0) * t + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2 + (p3 - p0 + p1 * 3.0 - p2 * 3.0) * t3) * 0.5
}

impl Track {
    pub fn cook() -> Track {
        let _z = forja::profiler::scope("cook.track");
        let pts: Vec<V2> = CONTROL.iter().map(|&(x, y)| v2(x, y)).collect();
        let n = pts.len();
        let mut dense = Vec::new();
        for i in 0..n {
            let p0 = pts[(i + n - 1) % n];
            let p1 = pts[i];
            let p2 = pts[(i + 1) % n];
            let p3 = pts[(i + 2) % n];
            for k in 0..400 {
                dense.push(catmull(p0, p1, p2, p3, k as f32 / 400.0));
            }
        }
        // reamostra por comprimento de arco
        let mut cum = vec![0.0f32; dense.len() + 1];
        for i in 0..dense.len() {
            cum[i + 1] = cum[i] + (dense[(i + 1) % dense.len()] - dense[i]).len();
        }
        let length = cum[dense.len()];
        let count = (length / STEP).floor() as usize;
        let step = length / count as f32;
        let mut pos = Vec::with_capacity(count);
        let mut j = 0;
        for k in 0..count {
            let target = k as f32 * step;
            while cum[j + 1] < target {
                j += 1;
            }
            let t = (target - cum[j]) / (cum[j + 1] - cum[j]).max(1e-6);
            pos.push(dense[j].lerp(dense[(j + 1) % dense.len()], t));
        }
        let tan: Vec<V2> = (0..count).map(|i| (pos[(i + 1) % count] - pos[(i + count - 1) % count]).norm()).collect();
        let raw: Vec<f32> = (0..count)
            .map(|i| {
                let a = tan[(i + count - 1) % count];
                let b = tan[(i + 1) % count];
                wrap_angle(b.y.atan2(b.x) - a.y.atan2(a.x)) / (2.0 * step)
            })
            .collect();
        let curv: Vec<f32> = (0..count).map(|i| (-6i64..=6).map(|d| raw[(i as i64 + d).rem_euclid(count as i64) as usize]).sum::<f32>() / 13.0).collect();
        let s = (0..count).map(|i| i as f32 * step).collect();
        Track { pos, tan, s, curv, length }
    }

    pub fn n(&self) -> usize {
        self.pos.len()
    }

    pub fn step(&self) -> f32 {
        self.length / self.n() as f32
    }

    pub fn normal(&self, i: usize) -> V2 {
        self.tan[i].perp()
    }

    pub fn wrap(&self, i: i64) -> usize {
        i.rem_euclid(self.n() as i64) as usize
    }

    pub fn idx_at_s(&self, s: f32) -> usize {
        ((s.rem_euclid(self.length) / self.step()) as usize).min(self.n() - 1)
    }

    /// Amostra mais próxima e deslocamento lateral (positivo = lado da normal).
    /// Busca local a partir da dica; cai para busca completa se a dica estiver longe.
    pub fn nearest(&self, p: V2, hint: u32) -> (usize, f32) {
        let n = self.n() as i64;
        let mut best = hint as usize % self.n();
        let mut bd = f32::MAX;
        for d in -40..=40 {
            let i = (hint as i64 + d).rem_euclid(n) as usize;
            let dd = (self.pos[i] - p).len2();
            if dd < bd {
                bd = dd;
                best = i;
            }
        }
        if bd > (WALL * 2.5) * (WALL * 2.5) {
            for i in 0..self.n() {
                let dd = (self.pos[i] - p).len2();
                if dd < bd {
                    bd = dd;
                    best = i;
                }
            }
        }
        (best, (p - self.pos[best]).dot(self.normal(best)))
    }

    /// Curvatura de maior módulo nos próximos `dist` metros (com sinal).
    pub fn curvature_ahead(&self, i: usize, dist: f32) -> f32 {
        let k = (dist / self.step()) as i64;
        let mut best: f32 = 0.0;
        for d in 0..k.max(1) {
            let c = self.curv[self.wrap(i as i64 + d)];
            if c.abs() > best.abs() {
                best = c;
            }
        }
        best
    }

    /// Menor distância entre trechos não vizinhos: se for menor que 2 muros, a pista se sobrepõe.
    pub fn min_clearance(&self) -> f32 {
        let n = self.n();
        let gap = (120.0 / self.step()) as usize;
        let mut best = f32::MAX;
        for i in (0..n).step_by(4) {
            for j in (0..n).step_by(4) {
                let d = (i as i64 - j as i64).unsigned_abs() as usize;
                if d.min(n - d) < gap {
                    continue;
                }
                best = best.min((self.pos[i] - self.pos[j]).len());
            }
        }
        best
    }
}

/// Posições do grid de largada: (pos, ângulo, índice na pista, s).
pub fn grid_positions() -> Vec<(V2, f32, u32, f32)> {
    let t = Track::cook();
    (0..CARS)
        .map(|i| {
            let s = t.length - 10.0 - i as f32 * 9.0;
            let idx = t.idx_at_s(s);
            let lat = if i % 2 == 0 { -3.5 } else { 3.5 };
            let p = t.pos[idx] + t.normal(idx) * lat;
            let a = t.tan[idx].y.atan2(t.tan[idx].x);
            (p, a, idx as u32, t.s[idx])
        })
        .collect()
}

fn label(s: &str) -> [u8; 12] {
    let mut l = [0u8; 12];
    for (i, b) in s.bytes().take(12).enumerate() {
        l[i] = b;
    }
    l
}

pub fn obstacle_layout() -> Vec<Obstacle> {
    let t = Track::cook();
    let mut rng = Rng::new(77);
    let mut out = Vec::new();
    let mut put = |p: V2, kind: u8, rng: &mut Rng| {
        let (radius, mass, color, name) = match kind {
            OBS_CONE => (0.45, 6.0, 0xFF7A1A, "cone"),
            OBS_BARREL => (0.7, 60.0, 0x2E6FD8, "barril"),
            _ => (1.4, f32::INFINITY, 0xB8B8B0, "bloco"),
        };
        out.push(Obstacle {
            pos: p,
            vel: V2::ZERO,
            radius,
            mass,
            kind,
            asleep: true,
            color,
            angle: rng.range(0.0, std::f32::consts::TAU),
            spin: 0.0,
            home: p,
            hits: 0,
            label: label(name),
        });
    };
    for (k, frac) in [0.10f32, 0.24, 0.37, 0.52, 0.66, 0.81, 0.93].iter().enumerate() {
        let i = t.idx_at_s(t.length * frac);
        let n = t.normal(i);
        let tg = t.tan[i];
        match k % 3 {
            0 => {
                // fileira de cones atravessando metade da pista
                let side = if k % 2 == 0 { 1.0 } else { -1.0 };
                for c in 0..6 {
                    put(t.pos[i] + n * (side * (1.5 + c as f32 * 1.4)) + tg * rng.range(-0.4, 0.4), OBS_CONE, &mut rng);
                }
            }
            1 => {
                // grupo de barris na borda + bloco de concreto num lado
                let side = if k % 2 == 0 { -1.0 } else { 1.0 };
                for c in 0..4 {
                    put(t.pos[i] + n * (side * 7.0) + tg * (c as f32 * 1.6), OBS_BARREL, &mut rng);
                }
                let j = t.wrap(i as i64 + 25);
                put(t.pos[j] + t.normal(j) * (-side * 4.0), OBS_BLOCK, &mut rng);
            }
            _ => {
                // chicane de cones
                for c in 0..8 {
                    let j = t.wrap(i as i64 + c * 3);
                    let side = if c % 2 == 0 { 1.0 } else { -1.0 };
                    put(t.pos[j] + t.normal(j) * (side * 5.5), OBS_CONE, &mut rng);
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ cenário assado

pub const SCENE_W: usize = 3072;
pub const M_PER_PX: f32 = 0.41;
const OUTSIDE: u32 = rgb(40, 74, 36);

#[derive(Clone)]
pub struct StaticScene {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Table for StaticScene {
    fn rows(&self) -> usize {
        self.px.len()
    }
}

impl StaticScene {
    #[inline]
    pub fn sample(&self, p: V2) -> u32 {
        let fx = p.x / M_PER_PX - 0.5;
        let fy = p.y / M_PER_PX - 0.5;
        let x0 = fx.floor() as i32;
        let y0 = fy.floor() as i32;
        if x0 < 0 || y0 < 0 || x0 >= self.w as i32 - 1 || y0 >= self.h as i32 - 1 {
            return OUTSIDE;
        }
        let tx = ((fx - x0 as f32) * 256.0) as u32;
        let ty = ((fy - y0 as f32) * 256.0) as u32;
        let i = y0 as usize * self.w + x0 as usize;
        let lerp_px = |a: u32, b: u32, t: u32| -> u32 {
            let it = 256 - t;
            let rb = (((a & 0xFF00FF) * it + (b & 0xFF00FF) * t) >> 8) & 0xFF00FF;
            let g = (((a & 0x00FF00) * it + (b & 0x00FF00) * t) >> 8) & 0x00FF00;
            rb | g
        };
        let top = lerp_px(self.px[i], self.px[i + 1], tx);
        let bot = lerp_px(self.px[i + self.w], self.px[i + self.w + 1], tx);
        lerp_px(top, bot, ty)
    }

    pub fn bake(t: &Track, pool: &JobPool) -> StaticScene {
        let _z = forja::profiler::scope("cook.scene");
        let w = SCENE_W;
        let h = SCENE_W;
        let mut px = vec![0u32; w * h];

        // 1. grama (paralelo por faixas)
        let rows = 96;
        pool.scope(|s| {
            for (band, chunk) in px.chunks_mut(rows * w).enumerate() {
                s.spawn(move |_| {
                    for (k, p) in chunk.iter_mut().enumerate() {
                        let x = (k % w) as i32;
                        let y = (band * rows + k / w) as i32;
                        let n1 = value_noise(x as f32 / 48.0, y as f32 / 48.0, 1);
                        let n2 = value_noise(x as f32 / 9.0, y as f32 / 9.0, 2);
                        let grain = (hash2(x, y, 3) & 15) as f32 - 8.0;
                        let stripe = if ((x + y) / 34) % 2 == 0 { 5.0 } else { -5.0 };
                        let f = 0.8 + 0.35 * n1;
                        let d = (n2 - 0.5) * 16.0 + grain + stripe;
                        *p = forja::draw::from_f(58.0 * f + d * 0.6, 108.0 * f + d, 46.0 * f + d * 0.5);
                    }
                });
            }
        });

        let mut c = Canvas::full(w, h, &mut px);
        let to_px = |p: V2| p / M_PER_PX;
        let n = t.n();
        let band = |c: &mut Canvas, a: f32, b: f32, color: &dyn Fn(usize, usize, usize) -> Option<(u32, f32)>| {
            for i in 0..n {
                let j = (i + 1) % n;
                let ni = t.normal(i);
                let nj = t.normal(j);
                let quad = [to_px(t.pos[i] + ni * a), to_px(t.pos[j] + nj * a), to_px(t.pos[j] + nj * b), to_px(t.pos[i] + ni * b)];
                c.poly_fn(&quad, |x, y| color(i, x, y));
            }
        };
        let grain = |x: usize, y: usize, s: u32| (hash2(x as i32, y as i32, s) & 15) as f32 - 7.5;

        // 2. área de escape (cascalho) e sombra externa dos muros
        band(&mut c, WALL + 1.6, WALL + 3.2, &|_, _, _| Some((0, 0.28)));
        band(&mut c, -WALL - 1.6, -WALL - 3.2, &|_, _, _| Some((0, 0.28)));
        band(&mut c, -16.0, 16.0, &|_, x, y| {
            let g = grain(x, y, 5);
            Some((forja::draw::from_f(190.0 + g, 168.0 + g, 122.0 + g * 0.8), 1.0))
        });
        // 3. asfalto com borda escura e desgaste no traçado
        band(&mut c, -9.7, 9.7, &|_, _, _| Some((rgb(44, 45, 50), 1.0)));
        band(&mut c, -HALF_WIDTH, HALF_WIDTH, &|i, x, y| {
            let g = grain(x, y, 7);
            let n = value_noise(x as f32 / 20.0, y as f32 / 20.0, 9);
            let wear = if t.curv[i].abs() > 0.004 { 6.0 } else { 0.0 };
            let v = 74.0 + g + (n - 0.5) * 10.0 - wear;
            Some((forja::draw::from_f(v, v + 1.0, v + 6.0), 1.0))
        });
        // 4. zebras nas curvas, linha branca nas retas
        for side in [-1.0f32, 1.0] {
            band(&mut c, side * HALF_WIDTH, side * (HALF_WIDTH + 1.5), &|i, _, _| {
                if t.curv[i].abs() > 0.0035 {
                    let red = ((t.s[i] / 3.0) as i32) % 2 == 0;
                    Some((if red { rgb(206, 38, 38) } else { rgb(236, 236, 236) }, 1.0))
                } else {
                    None
                }
            });
            band(&mut c, side * (HALF_WIDTH - 0.6), side * (HALF_WIDTH - 0.3), &|_, _, _| Some((rgb(230, 230, 225), 0.85)));
            // muro de pneus
            band(&mut c, side * WALL, side * (WALL + 1.6), &|i, _, _| {
                let k = ((t.s[i] / 2.0) as i32) % 6;
                let col = match k {
                    0 => rgb(200, 40, 40),
                    3 => rgb(235, 235, 235),
                    _ => rgb(30, 30, 34),
                };
                Some((col, 1.0))
            });
        }
        // 5. faixa central tracejada
        band(&mut c, -0.2, 0.2, &|i, _, _| if (t.s[i] % 10.0) < 5.0 { Some((rgb(240, 240, 230), 0.75)) } else { None });
        // 6. largada xadrez
        for i in 0..2 {
            let ni = t.normal(i);
            let p = t.pos[i];
            for k in -9..9 {
                for r in 0..2 {
                    let a = k as f32;
                    let base = p + t.tan[i] * (r as f32 * 1.0);
                    let quad = [to_px(base + ni * a), to_px(base + ni * (a + 1.0)), to_px(base + ni * (a + 1.0) + t.tan[i]), to_px(base + ni * a + t.tan[i])];
                    let white = (k + r + i as i32) % 2 == 0;
                    c.poly(&quad, if white { 0xF4F4F4 } else { 0x111111 }, 1.0);
                }
            }
        }
        // 7. arquibancada ao lado da largada
        {
            let i0 = t.idx_at_s(t.length - 60.0);
            let i1 = t.idx_at_s(t.length - 4.0).max(i0 + 1);
            for i in i0..i1 {
                let ni = t.normal(i);
                let nj = t.normal(i + 1);
                let quad = [to_px(t.pos[i] + ni * 27.0), to_px(t.pos[i + 1] + nj * 27.0), to_px(t.pos[i + 1] + nj * 36.0), to_px(t.pos[i] + ni * 36.0)];
                c.poly_fn(&quad, |x, y| {
                    let hsh = hash2(x as i32 / 2, y as i32 / 2, 11);
                    let crowd = [0xE11D48, 0xFACC15, 0x2563EB, 0xF4F4F4, 0x10B981, 0x8B5CF6];
                    Some((if hsh % 3 == 0 { rgb(70, 70, 78) } else { crowd[(hsh >> 4) as usize % crowd.len()] }, 1.0))
                });
            }
        }
        // 8. árvores e prédios fora dos muros
        let mut rng = Rng::new(4242);
        let world_size = w as f32 * M_PER_PX;
        let dist_track = |p: V2| -> f32 { t.pos.iter().step_by(3).map(|q| (*q - p).len2()).fold(f32::MAX, f32::min).sqrt() };
        for _ in 0..90 {
            let p = v2(rng.range(20.0, world_size - 20.0), rng.range(20.0, world_size - 20.0));
            let wd = rng.range(14.0, 34.0);
            let hd = rng.range(12.0, 28.0);
            if dist_track(p) < WALL + 12.0 + wd.max(hd) {
                continue;
            }
            let roof = rng.pick(&[rgb(120, 124, 132), rgb(150, 92, 70), rgb(88, 110, 140), rgb(170, 168, 160)]);
            let quad = |o: V2| [to_px(p + o), to_px(p + o + v2(wd, 0.0)), to_px(p + o + v2(wd, hd)), to_px(p + o + v2(0.0, hd))];
            c.poly(&quad(v2(3.0, 3.0)), 0, 0.35);
            c.poly(&quad(V2::ZERO), roof, 1.0);
            c.poly(&quad(v2(wd * 0.5, 0.0)), scale(roof, 0.85), 0.5);
        }
        for _ in 0..900 {
            let p = v2(rng.range(5.0, world_size - 5.0), rng.range(5.0, world_size - 5.0));
            let r = rng.range(2.0, 4.8);
            if dist_track(p) < WALL + 4.0 + r {
                continue;
            }
            let g = rng.range(0.75, 1.15);
            let leaf = forja::draw::from_f(46.0 * g, 96.0 * g, 40.0 * g);
            c.circle(to_px(p + v2(1.6, 1.8)), r / M_PER_PX, 0, 0.35);
            c.circle(to_px(p), r / M_PER_PX, leaf, 1.0);
            c.circle(to_px(p - v2(r * 0.3, r * 0.3)), r * 0.55 / M_PER_PX, mix(leaf, 0xC8F08C, 0.3), 0.55);
        }
        StaticScene { w, h, px }
    }
}
