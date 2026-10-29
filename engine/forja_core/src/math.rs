//! Tipos matemáticos simples: structs `repr(C)` + funções livres.

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

pub const VEC3_ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
pub const VEC3_X: Vec3 = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
pub const VEC3_Y: Vec3 = Vec3 { x: 0.0, y: 1.0, z: 0.0 };
pub const VEC3_Z: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };

// ---------------------------------------------------------------- f32

pub fn f32_clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

pub fn f32_lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---------------------------------------------------------------- Vec2

pub fn vec2_add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 { x: a.x + b.x, y: a.y + b.y }
}

pub fn vec2_sub(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 { x: a.x - b.x, y: a.y - b.y }
}

pub fn vec2_scale(a: Vec2, s: f32) -> Vec2 {
    Vec2 { x: a.x * s, y: a.y * s }
}

pub fn vec2_dot(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.y * b.y
}

pub fn vec2_length(a: Vec2) -> f32 {
    vec2_dot(a, a).sqrt()
}

// ---------------------------------------------------------------- Vec3

pub fn vec3_add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }
}

pub fn vec3_sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.x - b.x, y: a.y - b.y, z: a.z - b.z }
}

pub fn vec3_scale(a: Vec3, s: f32) -> Vec3 {
    Vec3 { x: a.x * s, y: a.y * s, z: a.z * s }
}

pub fn vec3_dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn vec3_cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub fn vec3_length(a: Vec3) -> f32 {
    vec3_dot(a, a).sqrt()
}

/// Vetor de comprimento 1. Comprimento zero (ou quase) devolve `VEC3_ZERO`.
pub fn vec3_normalize(a: Vec3) -> Vec3 {
    let len = vec3_length(a);
    if len <= 1.0e-8 {
        return VEC3_ZERO;
    }
    vec3_scale(a, 1.0 / len)
}

pub fn vec3_lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    Vec3 {
        x: f32_lerp(a.x, b.x, t),
        y: f32_lerp(a.y, b.y, t),
        z: f32_lerp(a.z, b.z, t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_segue_mao_direita() {
        assert_eq!(vec3_cross(VEC3_X, VEC3_Y), VEC3_Z);
        assert_eq!(vec3_cross(VEC3_Y, VEC3_Z), VEC3_X);
    }

    #[test]
    fn normalize_de_zero_nao_gera_nan() {
        assert_eq!(vec3_normalize(VEC3_ZERO), VEC3_ZERO);
        let n = vec3_normalize(Vec3 { x: 3.0, y: 0.0, z: 4.0 });
        assert!((vec3_length(n) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn clamp_e_lerp() {
        assert_eq!(f32_clamp(5.0, 0.0, 1.0), 1.0);
        assert_eq!(f32_clamp(-5.0, 0.0, 1.0), 0.0);
        assert_eq!(f32_lerp(2.0, 4.0, 0.5), 3.0);
        assert_eq!(vec2_length(Vec2 { x: 3.0, y: 4.0 }), 5.0);
    }
}
