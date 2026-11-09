//! Operadores dos vetores, gerados uma vez para todos.
//!
//! Os operadores de um vetor de `f32` sao sempre os mesmos, so muda a lista
//! de componentes. Escritos a mao em cada tipo, seriam dezenas de blocos
//! iguais; um erro num deles (um `-` no lugar de um `+`) passaria facil.

/// Implementa, componente a componente, para o vetor `$v` de campos `$f`:
///
/// - `+ - * /` entre dois vetores;
/// - `* /` com escalar, e `escalar * vetor`;
/// - negacao;
/// - `+= -=` com vetor, `*= /=` com escalar.
macro_rules! vector_ops {
    ($v:ident { $($f:ident),+ }) => {
        impl ::core::ops::Add for $v {
            type Output = $v;
            #[inline]
            fn add(self, other: $v) -> $v {
                $v { $($f: self.$f + other.$f),+ }
            }
        }

        impl ::core::ops::Sub for $v {
            type Output = $v;
            #[inline]
            fn sub(self, other: $v) -> $v {
                $v { $($f: self.$f - other.$f),+ }
            }
        }

        impl ::core::ops::Mul for $v {
            type Output = $v;
            #[inline]
            fn mul(self, other: $v) -> $v {
                $v { $($f: self.$f * other.$f),+ }
            }
        }

        impl ::core::ops::Div for $v {
            type Output = $v;
            #[inline]
            fn div(self, other: $v) -> $v {
                $v { $($f: self.$f / other.$f),+ }
            }
        }

        impl ::core::ops::Mul<f32> for $v {
            type Output = $v;
            #[inline]
            fn mul(self, scale: f32) -> $v {
                $v { $($f: self.$f * scale),+ }
            }
        }

        impl ::core::ops::Mul<$v> for f32 {
            type Output = $v;
            #[inline]
            fn mul(self, v: $v) -> $v {
                v * self
            }
        }

        impl ::core::ops::Div<f32> for $v {
            type Output = $v;
            #[inline]
            fn div(self, divisor: f32) -> $v {
                $v { $($f: self.$f / divisor),+ }
            }
        }

        impl ::core::ops::Neg for $v {
            type Output = $v;
            #[inline]
            fn neg(self) -> $v {
                $v { $($f: -self.$f),+ }
            }
        }

        impl ::core::ops::AddAssign for $v {
            #[inline]
            fn add_assign(&mut self, other: $v) {
                *self = *self + other;
            }
        }

        impl ::core::ops::SubAssign for $v {
            #[inline]
            fn sub_assign(&mut self, other: $v) {
                *self = *self - other;
            }
        }

        impl ::core::ops::MulAssign<f32> for $v {
            #[inline]
            fn mul_assign(&mut self, scale: f32) {
                *self = *self * scale;
            }
        }

        impl ::core::ops::DivAssign<f32> for $v {
            #[inline]
            fn div_assign(&mut self, divisor: f32) {
                *self = *self / divisor;
            }
        }
    };
}

pub(crate) use vector_ops;
