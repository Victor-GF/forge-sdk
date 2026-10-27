//! 8-lane SIMD types: `F32x8`, `I32x8` and `M32x8` (lane mask).
//!
//! Two implementations with the same API:
//! - **AVX2 + FMA** (`__m256`), selected when the crate is compiled for x86-64-v3 or newer. This
//!   repository sets that floor in `.cargo/config.toml`; `cpu::check_floor()` verifies at startup
//!   that the machine actually has it.
//! - **portable** (`[f32; 8]` loops the compiler can auto-vectorize), used everywhere else
//!   (other architectures, or a dependent crate built without the flag). Always compiled, and the
//!   tests check both agree.
//!
//! Code is written once against this API, 8 elements at a time, in structure-of-arrays form.

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"))]
mod avx2;
pub mod portable;

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"))]
pub use avx2::{F32x8, I32x8, M32x8};
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma")))]
pub use portable::{F32x8, I32x8, M32x8};

pub const LANES: usize = 8;

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"))]
pub const IMPLEMENTATION: &str = "avx2+fma";
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma")))]
pub const IMPLEMENTATION: &str = "portable";

/// Math built only on the primitive ops, implemented once for both backends.
macro_rules! shared_math {
    ($m:ident) => {
        impl $m::F32x8 {
            /// e^x, Cephes-style: range reduction to 2^n * e^r, degree-6 polynomial. Rel. error < 2e-7.
            #[inline(always)]
            pub fn exp(self) -> $m::F32x8 {
                use $m::{F32x8, I32x8};
                let x = self.max(F32x8::splat(-87.3)).min(F32x8::splat(88.7));
                let fx = x.mul_add(F32x8::splat(std::f32::consts::LOG2_E), F32x8::splat(0.5)).floor();
                let x = x - fx * F32x8::splat(0.693_359_4) - fx * F32x8::splat(-2.121_944_4e-4);
                let mut y = F32x8::splat(1.987_569_1e-4);
                for c in [1.398_199_9e-3, 8.333_452e-3, 4.166_579_6e-2, 1.666_666_5e-1, 0.5] {
                    y = y.mul_add(x, F32x8::splat(c));
                }
                let y = y.mul_add(x * x, x + F32x8::splat(1.0));
                let pow2n = F32x8::from_bits((fx.to_i32() + I32x8::splat(127)).shl::<23>());
                y * pow2n
            }

            /// self^n for a small non-negative integer exponent (binary exponentiation).
            #[inline(always)]
            pub fn powi(self, mut n: u32) -> $m::F32x8 {
                let (mut base, mut acc) = (self, $m::F32x8::splat(1.0));
                while n > 0 {
                    if n & 1 != 0 {
                        acc = acc * base;
                    }
                    base = base * base;
                    n >>= 1;
                }
                acc
            }

            /// a + (b - a) * t
            #[inline(always)]
            pub fn lerp(a: $m::F32x8, b: $m::F32x8, t: $m::F32x8) -> $m::F32x8 {
                (b - a).mul_add(t, a)
            }
        }
    };
}

shared_math!(portable);
#[cfg(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma"))]
shared_math!(avx2);

#[cfg(test)]
mod tests {
    use super::portable as p;
    use super::*;
    use crate::math::Rng;

    fn rand8(rng: &mut Rng, lo: f32, hi: f32) -> [f32; 8] {
        std::array::from_fn(|_| rng.range(lo, hi))
    }

    fn close(a: [f32; 8], b: [f32; 8], tol: f32) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol * (1.0 + x.abs().max(y.abs())))
    }

    #[test]
    fn active_implementation_matches_portable() {
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            let (a, b, c) = (rand8(&mut rng, -10.0, 10.0), rand8(&mut rng, -10.0, 10.0), rand8(&mut rng, 0.1, 10.0));
            let (fa, fb, fc) = (F32x8::from_array(a), F32x8::from_array(b), F32x8::from_array(c));
            let (pa, pb, pc) = (p::F32x8::from_array(a), p::F32x8::from_array(b), p::F32x8::from_array(c));
            assert!(close((fa + fb).to_array(), (pa + pb).to_array(), 1e-6));
            assert!(close((fa * fb - fc).to_array(), (pa * pb - pc).to_array(), 1e-5));
            assert!(close((fa / fc).to_array(), (pa / pc).to_array(), 1e-6));
            assert!(close(fa.mul_add(fb, fc).to_array(), pa.mul_add(pb, pc).to_array(), 1e-5));
            assert!(close(fc.sqrt().to_array(), pc.sqrt().to_array(), 1e-6));
            assert!(close(fa.floor().to_array(), pa.floor().to_array(), 0.0));
            assert!(close(fa.round().to_array(), pa.round().to_array(), 0.0));
            assert!(close(fa.exp().to_array(), pa.exp().to_array(), 1e-6));
            assert_eq!(fa.lt(fb).bits(), pa.lt(pb).bits());
            assert_eq!(fa.ge(fb).bits(), pa.ge(pb).bits());
            let m = fa.lt(fb);
            assert_eq!(m.select(fa, fc).to_array(), p::M32x8::from_bits(m.bits()).select(pa, pc).to_array());
            assert_eq!(fa.to_i32().to_array(), pa.to_i32().to_array());
            let ia: [i32; 8] = std::array::from_fn(|i| (a[i] * 1000.0) as i32);
            let ib: [i32; 8] = std::array::from_fn(|i| (b[i] * 1000.0) as i32);
            let (ja, jb) = (I32x8::from_array(ia), I32x8::from_array(ib));
            let (qa, qb) = (p::I32x8::from_array(ia), p::I32x8::from_array(ib));
            assert_eq!((ja * jb + ja - jb).to_array(), (qa * qb + qa - qb).to_array());
            assert_eq!((ja.shl::<3>() | jb).to_array(), (qa.shl::<3>() | qb).to_array());
            assert_eq!(ja.min(jb).max(I32x8::splat(-5)).to_array(), qa.min(qb).max(p::I32x8::splat(-5)).to_array());
            assert_eq!(ja.gt(jb).bits(), qa.gt(qb).bits());
            assert_eq!(fa.sum(), pa.sum());
        }
    }

    #[test]
    fn exp_is_accurate() {
        let mut rng = Rng::new(9);
        for _ in 0..1000 {
            let a = rand8(&mut rng, -20.0, 20.0);
            let e = F32x8::from_array(a).exp().to_array();
            for i in 0..8 {
                let want = a[i].exp();
                assert!((e[i] - want).abs() <= 2e-6 * want, "exp({}) = {} want {}", a[i], e[i], want);
            }
        }
    }

    #[test]
    fn gather_masks_and_loads() {
        let data: Vec<f32> = (0..100).map(|i| i as f32 * 0.5).collect();
        let idx = I32x8::from_array([0, 99, 3, 50, 7, 7, 1, 2]);
        assert_eq!(F32x8::gather(&data, idx).to_array(), [0.0, 49.5, 1.5, 25.0, 3.5, 3.5, 0.5, 1.0]);
        let words: Vec<u32> = (0..10).map(|i| i * 3).collect();
        assert_eq!(I32x8::gather_u32(&words, I32x8::from_array([9, 0, 1, 2, 3, 4, 5, 6])).to_array(), [27, 0, 3, 6, 9, 12, 15, 18]);
        let m = F32x8::ramp(0.0).lt(F32x8::splat(3.0));
        assert_eq!((m.bits(), m.count(), m.any(), m.all()), (0b111, 3, true, false));
        assert_eq!((!m).bits(), 0b1111_1000);
        let mut out = [0u32; 8];
        m.select_i32(I32x8::splat(1), I32x8::splat(2)).store_u32(&mut out);
        assert_eq!(out, [1, 1, 1, 2, 2, 2, 2, 2]);
    }

    #[test]
    #[should_panic]
    fn gather_out_of_bounds_panics() {
        let data = [1.0f32; 4];
        F32x8::gather(&data, I32x8::ramp(0));
    }
}
