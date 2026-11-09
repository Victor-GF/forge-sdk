//! Vetor de 4 componentes `f32`: ponto ou direcao em coordenadas homogeneas
//! (para multiplicar por matriz 4x4), cor RGBA, plano (`ax + by + cz + d`).
//!
//! # Alinhamento a 16 bytes
//!
//! Diferente do `Vec3`, o `Vec4` e alinhado a 16, sem custo de tamanho: sao
//! 16 bytes de qualquer jeito. Com isso:
//! - Cabe exatamente num registro SIMD (SSE `__m128`, NEON `float32x4_t`),
//!   com carga alinhada: o compilador pode fazer `+ - * /` numa instrucao.
//! - Bate com o alinhamento de `float4`/`vec4` em buffers de constantes da
//!   GPU (HLSL `cbuffer`, GLSL `std140`).
//!
//! O preco: um `&[f32]` qualquer nao pode ser reinterpretado como `&[Vec4]`,
//! porque o endereco pode nao ser multiplo de 16. Converta com `From`.

use crate::Vec3;
use crate::ops::vector_ops;
use crate::scalar;
use forge_core::layout;

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

layout!(Vec4, size = 16, align = 16);

impl Vec4 {
    pub const ZERO: Vec4 = Vec4::splat(0.0);
    pub const ONE: Vec4 = Vec4::splat(1.0);

    pub const X: Vec4 = Vec4::new(1.0, 0.0, 0.0, 0.0);
    pub const Y: Vec4 = Vec4::new(0.0, 1.0, 0.0, 0.0);
    pub const Z: Vec4 = Vec4::new(0.0, 0.0, 1.0, 0.0);
    pub const W: Vec4 = Vec4::new(0.0, 0.0, 0.0, 1.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Vec4 {
        Vec4 { x, y, z, w }
    }

    /// Os quatro componentes com o mesmo valor.
    #[inline]
    pub const fn splat(value: f32) -> Vec4 {
        Vec4::new(value, value, value, value)
    }

    #[inline]
    pub const fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// Os tres primeiros componentes, descartando `w`. Para um ponto
    /// homogeneo depois de uma projecao, divida por `w` antes (`project`).
    #[inline]
    pub const fn truncate(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// Ponto homogeneo de volta ao espaco 3D: `xyz / w`. E o passo depois
    /// da matriz de projecao. `w` nao pode ser zero (direcao, nao ponto):
    /// em debug, panic; em release, infinito ou NaN.
    #[inline]
    pub fn project(self) -> Vec3 {
        debug_assert!(self.w != 0.0, "project com w = 0: e uma direcao, nao um ponto");
        self.truncate() / self.w
    }

    /// Produto escalar dos quatro componentes.
    #[inline]
    pub fn dot(self, other: Vec4) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z + self.w * other.w
    }

    /// Comprimento ao quadrado, sem raiz: para comparar, basta.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        scalar::sqrt(self.length_squared())
    }

    #[inline]
    pub fn distance(self, other: Vec4) -> f32 {
        (other - self).length()
    }

    #[inline]
    pub fn distance_squared(self, other: Vec4) -> f32 {
        (other - self).length_squared()
    }

    /// Mesma direcao, comprimento 1, contando os quatro componentes. O vetor
    /// nao pode ser nulo: em debug, panic; em release, NaN.
    #[inline]
    pub fn normalize(self) -> Vec4 {
        let length = self.length();
        debug_assert!(length > 0.0, "normalize de vetor nulo");
        self / length
    }

    /// Como `normalize`, mas um vetor nulo (ou pequeno demais para ter
    /// direcao em `f32`) volta `ZERO`.
    #[inline]
    pub fn normalize_or_zero(self) -> Vec4 {
        let length_squared = self.length_squared();
        if length_squared > f32::MIN_POSITIVE {
            self / scalar::sqrt(length_squared)
        } else {
            Vec4::ZERO
        }
    }

    /// Interpolacao linear: `self` com `t = 0`, `to` com `t = 1`. Entre duas
    /// cores RGBA, mistura inclusive a transparencia.
    #[inline]
    pub fn lerp(self, to: Vec4, t: f32) -> Vec4 {
        self + (to - self) * t
    }

    /// Menor de cada componente.
    #[inline]
    pub fn min(self, other: Vec4) -> Vec4 {
        Vec4::new(
            self.x.min(other.x),
            self.y.min(other.y),
            self.z.min(other.z),
            self.w.min(other.w),
        )
    }

    /// Maior de cada componente.
    #[inline]
    pub fn max(self, other: Vec4) -> Vec4 {
        Vec4::new(
            self.x.max(other.x),
            self.y.max(other.y),
            self.z.max(other.z),
            self.w.max(other.w),
        )
    }
}

impl From<[f32; 4]> for Vec4 {
    #[inline]
    fn from([x, y, z, w]: [f32; 4]) -> Vec4 {
        Vec4::new(x, y, z, w)
    }
}

impl From<Vec4> for [f32; 4] {
    #[inline]
    fn from(v: Vec4) -> [f32; 4] {
        v.to_array()
    }
}

// Operadores: entre dois vetores, componente a componente; com escalar, o
// escalar vale para os quatro componentes.
vector_ops!(Vec4 { x, y, z, w });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operadores() {
        let a = Vec4::new(1.0, 2.0, 3.0, 4.0);
        let b = Vec4::new(5.0, 6.0, 7.0, 8.0);
        assert_eq!(a + b, Vec4::new(6.0, 8.0, 10.0, 12.0));
        assert_eq!(b - a, Vec4::splat(4.0));
        assert_eq!(a * 2.0, Vec4::new(2.0, 4.0, 6.0, 8.0));
        assert_eq!(2.0 * a, a * 2.0);
        assert_eq!(a * b, Vec4::new(5.0, 12.0, 21.0, 32.0));
        assert_eq!(b / 2.0, Vec4::new(2.5, 3.0, 3.5, 4.0));
        assert_eq!(a / a, Vec4::ONE);
        assert_eq!(-a, Vec4::new(-1.0, -2.0, -3.0, -4.0));

        let mut c = a;
        c += b;
        c -= a;
        c *= 2.0;
        c /= 4.0;
        assert_eq!(c, b / 2.0);
    }

    #[test]
    fn dot_e_comprimento() {
        assert_eq!(Vec4::X.dot(Vec4::W), 0.0);
        assert_eq!(Vec4::new(1.0, 2.0, 3.0, 4.0).dot(Vec4::ONE), 10.0);
        let v = Vec4::new(1.0, 1.0, 1.0, 1.0);
        assert_eq!(v.length_squared(), 4.0);
        assert_eq!(v.length(), 2.0);
        assert_eq!(Vec4::ZERO.distance(Vec4::new(0.0, 3.0, 0.0, 4.0)), 5.0);
        assert_eq!(Vec4::ZERO.distance_squared(v), 4.0);
    }

    #[test]
    fn normalize() {
        assert_eq!(Vec4::new(0.0, 0.0, 0.0, 2.0).normalize(), Vec4::W);
        assert_eq!(Vec4::new(2.0, 2.0, 2.0, 2.0).normalize(), Vec4::splat(0.5));
        assert_eq!(Vec4::ZERO.normalize_or_zero(), Vec4::ZERO);
    }

    #[test]
    fn homogeneo_e_vec3() {
        let p = Vec4::new(2.0, 4.0, 6.0, 2.0);
        assert_eq!(p.truncate(), Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(p.project(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(Vec3::new(1.0, 2.0, 3.0).extend(1.0).project(), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn lerp_min_max() {
        let transparente = Vec4::new(1.0, 0.0, 0.0, 0.0);
        let opaco = Vec4::new(0.0, 0.0, 1.0, 1.0);
        assert_eq!(transparente.lerp(opaco, 0.5), Vec4::new(0.5, 0.0, 0.5, 0.5));
        assert_eq!(transparente.min(opaco), Vec4::ZERO);
        assert_eq!(transparente.max(opaco), Vec4::new(1.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn conversao_de_array() {
        let v: Vec4 = [1.0, 2.0, 3.0, 4.0].into();
        assert_eq!(v, Vec4::new(1.0, 2.0, 3.0, 4.0));
        assert_eq!(<[f32; 4]>::from(v), [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn alinhado_a_16() {
        let vetores = [Vec4::ZERO; 3];
        for v in &vetores {
            assert_eq!(v as *const Vec4 as usize % 16, 0);
        }
    }
}
