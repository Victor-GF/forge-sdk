//! Tipos matemáticos simples: structs `repr(C)` com métodos próprios (sem traits de operador).
//!
//! Os nomes evitam `add`/`sub`/`mul` para não parecerem implementações das traits de operador.

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

// ---------------------------------------------------------------- escalares

/// Limita `v` a `[min, max]`. Diferente de `f32::clamp`, não entra em pânico se `min > max`.
pub fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---------------------------------------------------------------- Vec2

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Vec2 {
        Vec2 { x, y }
    }

    pub fn plus(self, b: Vec2) -> Vec2 {
        Vec2 { x: self.x + b.x, y: self.y + b.y }
    }

    pub fn minus(self, b: Vec2) -> Vec2 {
        Vec2 { x: self.x - b.x, y: self.y - b.y }
    }

    pub fn scaled(self, s: f32) -> Vec2 {
        Vec2 { x: self.x * s, y: self.y * s }
    }

    pub fn dot(self, b: Vec2) -> f32 {
        self.x * b.x + self.y * b.y
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
}

// ---------------------------------------------------------------- Vec3

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    pub const X: Vec3 = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
    pub const Y: Vec3 = Vec3 { x: 0.0, y: 1.0, z: 0.0 };
    pub const Z: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };

    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    pub fn plus(self, b: Vec3) -> Vec3 {
        Vec3 { x: self.x + b.x, y: self.y + b.y, z: self.z + b.z }
    }

    pub fn minus(self, b: Vec3) -> Vec3 {
        Vec3 { x: self.x - b.x, y: self.y - b.y, z: self.z - b.z }
    }

    pub fn scaled(self, s: f32) -> Vec3 {
        Vec3 { x: self.x * s, y: self.y * s, z: self.z * s }
    }

    pub fn dot(self, b: Vec3) -> f32 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }

    pub fn cross(self, b: Vec3) -> Vec3 {
        Vec3 {
            x: self.y * b.z - self.z * b.y,
            y: self.z * b.x - self.x * b.z,
            z: self.x * b.y - self.y * b.x,
        }
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Vetor de comprimento 1. Comprimento zero (ou quase) devolve `Vec3::ZERO`.
    pub fn normalized(self) -> Vec3 {
        let len = self.length();
        if len <= 1.0e-8 {
            return Vec3::ZERO;
        }
        self.scaled(1.0 / len)
    }

    pub fn lerp(self, b: Vec3, t: f32) -> Vec3 {
        Vec3 { x: lerp(self.x, b.x, t), y: lerp(self.y, b.y, t), z: lerp(self.z, b.z, t) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_is_right_handed() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
    }

    #[test]
    fn normalizing_zero_does_not_produce_nan() {
        assert_eq!(Vec3::ZERO.normalized(), Vec3::ZERO);
        let n = Vec3::new(3.0, 0.0, 4.0).normalized();
        assert!((n.length() - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn clamp_and_lerp() {
        assert_eq!(clamp(5.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp(-5.0, 0.0, 1.0), 0.0);
        assert_eq!(lerp(2.0, 4.0, 0.5), 3.0);
        assert_eq!(Vec2::new(3.0, 4.0).length(), 5.0);
        assert_eq!(Vec3::new(1.0, 2.0, 3.0).plus(Vec3::X).minus(Vec3::Y), Vec3::new(2.0, 1.0, 3.0));
    }
}
