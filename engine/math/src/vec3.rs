//! Vetor de 3 componentes `f32`: posicao, direcao, velocidade, escala, cor.
//!
//! 12 bytes, sem SIMD. Um registro SSE tem 4 lanes e um `Vec3` ocupa 3: a
//! versao alinhada a 16 bytes, com uma lane de sobra, so compensa em lote, e
//! fica para quando um perfil de desempenho pedir.

use crate::Vec4;
use crate::ops::vector_ops;
use crate::scalar;
use forge_core::layout;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

layout!(Vec3, size = 12, align = 4);

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::splat(0.0);
    pub const ONE: Vec3 = Vec3::splat(1.0);

    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    /// Direcoes da convencao da engine (ver o crate).
    pub const RIGHT: Vec3 = Vec3::X;
    pub const UP: Vec3 = Vec3::Y;
    pub const FORWARD: Vec3 = Vec3::new(0.0, 0.0, -1.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// Os tres componentes com o mesmo valor.
    #[inline]
    pub const fn splat(value: f32) -> Vec3 {
        Vec3::new(value, value, value)
    }

    #[inline]
    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// Com um quarto componente: `w = 1` para um ponto (a translacao de uma
    /// matriz o move), `w = 0` para uma direcao (nao move).
    #[inline]
    pub const fn extend(self, w: f32) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, w)
    }

    /// Produto escalar: `|a| |b| cos(angulo)`. Zero com vetores
    /// perpendiculares; positivo se apontam para o mesmo lado.
    #[inline]
    pub fn dot(self, other: Vec3) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Produto vetorial: perpendicular aos dois, pela regra da mao direita
    /// (`X.cross(Y) == Z`). O comprimento e a area do paralelogramo entre
    /// eles.
    #[inline]
    pub fn cross(self, other: Vec3) -> Vec3 {
        Vec3::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    /// Comprimento ao quadrado, sem raiz: para comparar distancias, basta.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        scalar::sqrt(self.length_squared())
    }

    #[inline]
    pub fn distance(self, other: Vec3) -> f32 {
        (other - self).length()
    }

    #[inline]
    pub fn distance_squared(self, other: Vec3) -> f32 {
        (other - self).length_squared()
    }

    /// Mesma direcao, comprimento 1. O vetor nao pode ser nulo: em debug,
    /// panic; em release, NaN. Se nulo for possivel, use
    /// `normalize_or_zero`.
    #[inline]
    pub fn normalize(self) -> Vec3 {
        let length = self.length();
        debug_assert!(length > 0.0, "normalize de vetor nulo");
        self / length
    }

    /// Como `normalize`, mas um vetor nulo (ou pequeno demais para ter
    /// direcao em `f32`) volta `ZERO`. Para direcoes que podem nao existir,
    /// como a do input com o analogico solto.
    #[inline]
    pub fn normalize_or_zero(self) -> Vec3 {
        let length_squared = self.length_squared();
        if length_squared > f32::MIN_POSITIVE {
            self / scalar::sqrt(length_squared)
        } else {
            Vec3::ZERO
        }
    }

    /// Interpolacao linear: `self` com `t = 0`, `to` com `t = 1`.
    #[inline]
    pub fn lerp(self, to: Vec3, t: f32) -> Vec3 {
        self + (to - self) * t
    }

    /// Menor de cada componente.
    #[inline]
    pub fn min(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x.min(other.x), self.y.min(other.y), self.z.min(other.z))
    }

    /// Maior de cada componente.
    #[inline]
    pub fn max(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x.max(other.x), self.y.max(other.y), self.z.max(other.z))
    }
}

impl From<[f32; 3]> for Vec3 {
    #[inline]
    fn from([x, y, z]: [f32; 3]) -> Vec3 {
        Vec3::new(x, y, z)
    }
}

impl From<Vec3> for [f32; 3] {
    #[inline]
    fn from(v: Vec3) -> [f32; 3] {
        v.to_array()
    }
}

// Operadores: entre dois vetores, componente a componente; com escalar, o
// escalar vale para os tres componentes.
vector_ops!(Vec3 { x, y, z });

#[cfg(test)]
mod tests {
    use super::*;

    /// Igualdade com folga de arredondamento de `f32`.
    fn close(a: Vec3, b: Vec3) -> bool {
        let d = a - b;
        let epsilon = 1e-6;
        d.x.max(-d.x) < epsilon && d.y.max(-d.y) < epsilon && d.z.max(-d.z) < epsilon
    }

    #[test]
    fn operadores() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(a + b, Vec3::new(5.0, 7.0, 9.0));
        assert_eq!(b - a, Vec3::splat(3.0));
        assert_eq!(a * 2.0, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(2.0 * a, a * 2.0);
        assert_eq!(a * b, Vec3::new(4.0, 10.0, 18.0));
        assert_eq!(b / 2.0, Vec3::new(2.0, 2.5, 3.0));
        assert_eq!(-a, Vec3::new(-1.0, -2.0, -3.0));

        let mut c = a;
        c += b;
        c -= a;
        c *= 2.0;
        c /= 4.0;
        assert_eq!(c, b / 2.0);
    }

    #[test]
    fn dot_perpendicular_e_paralelo() {
        assert_eq!(Vec3::X.dot(Vec3::Y), 0.0);
        assert_eq!(Vec3::new(1.0, 2.0, 3.0).dot(Vec3::new(4.0, 5.0, 6.0)), 32.0);
        assert!(Vec3::X.dot(-Vec3::X) < 0.0);
    }

    #[test]
    fn cross_e_destro() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
        assert_eq!(Vec3::Z.cross(Vec3::X), Vec3::Y);
        assert_eq!(Vec3::Y.cross(Vec3::X), -Vec3::Z);
        // Direita x cima = para tras (+Z): a frente e -Z.
        assert_eq!(Vec3::RIGHT.cross(Vec3::UP), -Vec3::FORWARD);
    }

    #[test]
    fn comprimento_e_distancia() {
        let v = Vec3::new(3.0, 4.0, 0.0);
        assert_eq!(v.length_squared(), 25.0);
        assert_eq!(v.length(), 5.0);
        assert_eq!(Vec3::ONE.distance(Vec3::new(1.0, 1.0, 3.0)), 2.0);
        assert_eq!(Vec3::ZERO.distance_squared(v), 25.0);
    }

    #[test]
    fn normalize() {
        let n = Vec3::new(0.0, 3.0, 4.0).normalize();
        assert!(close(n, Vec3::new(0.0, 0.6, 0.8)));
        let error = n.length() - 1.0;
        assert!(error.max(-error) < 1e-6);
        assert_eq!(Vec3::ZERO.normalize_or_zero(), Vec3::ZERO);
        assert_eq!(Vec3::new(0.0, 0.0, -2.0).normalize_or_zero(), Vec3::FORWARD);
    }

    #[test]
    fn lerp_min_max() {
        let a = Vec3::new(0.0, 10.0, -2.0);
        let b = Vec3::new(2.0, 20.0, 2.0);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        assert_eq!(a.lerp(b, 0.5), Vec3::new(1.0, 15.0, 0.0));
        assert_eq!(a.min(b), a);
        assert_eq!(Vec3::new(5.0, 0.0, 1.0).max(Vec3::new(1.0, 3.0, 1.0)), Vec3::new(5.0, 3.0, 1.0));
    }

    #[test]
    fn extend_para_vec4() {
        assert_eq!(Vec3::new(1.0, 2.0, 3.0).extend(1.0), Vec4::new(1.0, 2.0, 3.0, 1.0));
    }

    #[test]
    fn conversao_de_array() {
        let v: Vec3 = [1.0, 2.0, 3.0].into();
        assert_eq!(v, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(<[f32; 3]>::from(v), [1.0, 2.0, 3.0]);
    }
}
