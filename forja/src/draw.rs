//! Rasterização por software. Um `Canvas` é uma faixa de linhas [y0, y1) de um
//! framebuffer maior: as coordenadas são sempre globais e tudo é recortado na faixa,
//! então N faixas podem desenhar a mesma cena em paralelo sem sincronização.

use crate::math::{v2, V2};

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

pub fn channels(c: u32) -> (f32, f32, f32) {
    (((c >> 16) & 0xFF) as f32, ((c >> 8) & 0xFF) as f32, (c & 0xFF) as f32)
}

pub fn from_f(r: f32, g: f32, b: f32) -> u32 {
    let q = |x: f32| x.clamp(0.0, 255.0) as u32;
    (q(r) << 16) | (q(g) << 8) | q(b)
}

pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    blend_px(a, b, t)
}

pub fn scale(c: u32, f: f32) -> u32 {
    let (r, g, b) = channels(c);
    from_f(r * f, g * f, b * f)
}

pub fn add(a: u32, b: u32, f: f32) -> u32 {
    let (r0, g0, b0) = channels(a);
    let (r1, g1, b1) = channels(b);
    from_f(r0 + r1 * f, g0 + g1 * f, b0 + b1 * f)
}

#[inline]
pub fn blend_px(dst: u32, src: u32, a: f32) -> u32 {
    let ai = (a.clamp(0.0, 1.0) * 256.0) as u32;
    let inv = 256 - ai;
    let rb = (((src & 0xFF00FF) * ai + (dst & 0xFF00FF) * inv) >> 8) & 0xFF00FF;
    let g = (((src & 0x00FF00) * ai + (dst & 0x00FF00) * inv) >> 8) & 0x00FF00;
    rb | g
}

pub struct Canvas<'a> {
    pub w: usize,
    pub h: usize,
    pub y0: usize,
    pub y1: usize,
    pub px: &'a mut [u32],
}

impl<'a> Canvas<'a> {
    pub fn full(w: usize, h: usize, px: &'a mut [u32]) -> Canvas<'a> {
        Canvas { w, h, y0: 0, y1: h, px }
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> usize {
        (y - self.y0) * self.w + x
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u32 {
        self.px[self.idx(x, y)]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, c: u32) {
        let i = self.idx(x, y);
        self.px[i] = c;
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: u32, a: f32) {
        if x < 0 || x >= self.w as i32 || y < self.y0 as i32 || y >= self.y1 as i32 || a <= 0.0 {
            return;
        }
        let i = self.idx(x as usize, y as usize);
        self.px[i] = if a >= 1.0 { c } else { blend_px(self.px[i], c, a) };
    }

    #[inline]
    pub fn add_px(&mut self, x: i32, y: i32, c: u32, f: f32) {
        if x < 0 || x >= self.w as i32 || y < self.y0 as i32 || y >= self.y1 as i32 {
            return;
        }
        let i = self.idx(x as usize, y as usize);
        self.px[i] = add(self.px[i], c, f);
    }

    /// Recorta um retângulo [x0,x1) x [y0,y1) na faixa. None se vazio.
    fn clip(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<(usize, usize, usize, usize)> {
        let x0 = x0.max(0) as usize;
        let x1 = x1.min(self.w as i32);
        let y0 = y0.max(self.y0 as i32) as usize;
        let y1 = y1.min(self.y1 as i32);
        if x1 <= x0 as i32 || y1 <= y0 as i32 {
            return None;
        }
        Some((x0, y0, x1 as usize, y1 as usize))
    }

    pub fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: u32, a: f32) {
        if let Some((x0, y0, x1, y1)) = self.clip(x0, y0, x1, y1) {
            for y in y0..y1 {
                let row = self.idx(0, y);
                for p in &mut self.px[row + x0..row + x1] {
                    *p = if a >= 1.0 { c } else { blend_px(*p, c, a) };
                }
            }
        }
    }

    pub fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: u32, a: f32) {
        self.fill_rect(x0, y0, x1, y0 + 1, c, a);
        self.fill_rect(x0, y1 - 1, x1, y1, c, a);
        self.fill_rect(x0, y0 + 1, x0 + 1, y1 - 1, c, a);
        self.fill_rect(x1 - 1, y0 + 1, x1, y1 - 1, c, a);
    }

    pub fn line(&mut self, a: V2, b: V2, c: u32, alpha: f32) {
        let d = b - a;
        let n = d.x.abs().max(d.y.abs()).ceil().max(1.0) as i32;
        if n > 4000 {
            return;
        }
        let step = d / n as f32;
        let mut p = a;
        for _ in 0..=n {
            self.blend(p.x as i32, p.y as i32, c, alpha);
            p += step;
        }
    }

    pub fn line_add(&mut self, a: V2, b: V2, c: u32, f: f32) {
        let d = b - a;
        let n = d.x.abs().max(d.y.abs()).ceil().max(1.0) as i32;
        if n > 4000 {
            return;
        }
        let step = d / n as f32;
        let mut p = a;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            self.add_px(p.x as i32, p.y as i32, c, f * t);
            p += step;
        }
    }

    /// Círculo preenchido com borda suave de 1 px.
    pub fn circle(&mut self, center: V2, r: f32, c: u32, alpha: f32) {
        if r <= 0.0 {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.clip(
            (center.x - r - 1.0) as i32,
            (center.y - r - 1.0) as i32,
            (center.x + r + 2.0) as i32,
            (center.y + r + 2.0) as i32,
        ) else {
            return;
        };
        for y in y0..y1 {
            let dy = y as f32 + 0.5 - center.y;
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - center.x;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (r - d + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let i = self.idx(x, y);
                    self.px[i] = blend_px(self.px[i], c, alpha * cov);
                }
            }
        }
    }

    pub fn ring(&mut self, center: V2, r: f32, thickness: f32, c: u32, alpha: f32) {
        let ro = r + thickness * 0.5;
        let ri = (r - thickness * 0.5).max(0.0);
        let Some((x0, y0, x1, y1)) = self.clip(
            (center.x - ro - 1.0) as i32,
            (center.y - ro - 1.0) as i32,
            (center.x + ro + 2.0) as i32,
            (center.y + ro + 2.0) as i32,
        ) else {
            return;
        };
        for y in y0..y1 {
            let dy = y as f32 + 0.5 - center.y;
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - center.x;
                let d = (dx * dx + dy * dy).sqrt();
                let cov = (ro - d + 0.5).clamp(0.0, 1.0) * (d - ri + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let i = self.idx(x, y);
                    self.px[i] = blend_px(self.px[i], c, alpha * cov);
                }
            }
        }
    }

    /// Polígono convexo preenchido (qualquer orientação), cor sólida.
    pub fn poly(&mut self, pts: &[V2], c: u32, alpha: f32) {
        self.poly_fn(pts, |_, _| Some((c, alpha)));
    }

    /// Polígono convexo com "shader" por pixel: f(x, y) -> (cor, alpha).
    pub fn poly_fn(&mut self, pts: &[V2], mut f: impl FnMut(usize, usize) -> Option<(u32, f32)>) {
        if pts.len() < 3 {
            return;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            minx = minx.min(p.x);
            miny = miny.min(p.y);
            maxx = maxx.max(p.x);
            maxy = maxy.max(p.y);
        }
        let Some((x0, y0, x1, y1)) = self.clip(minx as i32, miny as i32, maxx as i32 + 1, maxy as i32 + 1) else {
            return;
        };
        // orientação: soma do produto vetorial
        let mut area = 0.0;
        for i in 0..pts.len() {
            area += pts[i].cross(pts[(i + 1) % pts.len()]);
        }
        let sign = if area >= 0.0 { 1.0 } else { -1.0 };
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            for x in x0..x1 {
                let p = v2(x as f32 + 0.5, py);
                let mut inside = true;
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    if (b - a).cross(p - a) * sign < 0.0 {
                        inside = false;
                        break;
                    }
                }
                if inside {
                    if let Some((c, a)) = f(x, y) {
                        let i = self.idx(x, y);
                        self.px[i] = if a >= 1.0 { c } else { blend_px(self.px[i], c, a) };
                    }
                }
            }
        }
    }

    /// Retângulo orientado com shader em coordenadas locais (u, v) em [-1, 1].
    /// `hu` e `hv` são os semi-eixos em pixels. f(u, v, x, y) -> (cor, alpha).
    pub fn quad_shader(&mut self, center: V2, hu: V2, hv: V2, mut f: impl FnMut(f32, f32, usize, usize) -> Option<(u32, f32)>) {
        let ext = v2(hu.x.abs() + hv.x.abs(), hu.y.abs() + hv.y.abs());
        let Some((x0, y0, x1, y1)) = self.clip(
            (center.x - ext.x - 1.0) as i32,
            (center.y - ext.y - 1.0) as i32,
            (center.x + ext.x + 2.0) as i32,
            (center.y + ext.y + 2.0) as i32,
        ) else {
            return;
        };
        let det = hu.x * hv.y - hu.y * hv.x;
        if det.abs() < 1e-6 {
            return;
        }
        let inv = 1.0 / det;
        for y in y0..y1 {
            let dy = y as f32 + 0.5 - center.y;
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - center.x;
                let u = (dx * hv.y - dy * hv.x) * inv;
                let v = (dy * hu.x - dx * hu.y) * inv;
                if u.abs() <= 1.0 && v.abs() <= 1.0 {
                    if let Some((c, a)) = f(u, v, x, y) {
                        let i = self.idx(x, y);
                        self.px[i] = if a >= 1.0 { c } else { blend_px(self.px[i], c, a) };
                    }
                }
            }
        }
    }
}
