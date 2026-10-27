//! AVX2 + FMA implementation. Only compiled when those features are enabled for the whole crate
//! (x86-64-v3 floor, checked at startup by `cpu::check_floor`).
//!
//! SAFETY (whole module): Rust still requires `unsafe` to call `#[target_feature]` intrinsics
//! even when the feature is enabled crate-wide; the `cfg` on this module guarantees it is. The
//! only other unsafety is memory access through raw pointers, each behind a length/index check.

use std::arch::x86_64::*;
use std::ops::{Add, BitAnd, BitOr, Div, Mul, Neg, Not, Sub};

#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct F32x8(__m256);
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct I32x8(__m256i);
/// Lane mask: every bit of a lane set, or none (the format compares produce).
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct M32x8(__m256);

#[inline(always)]
fn in_bounds(idx: __m256i, len: usize) -> bool {
    unsafe {
        assert!(len <= i32::MAX as usize, "gather base too large for 32-bit indices");
        let ok = _mm256_and_si256(
            _mm256_cmpgt_epi32(idx, _mm256_set1_epi32(-1)),
            _mm256_cmpgt_epi32(_mm256_set1_epi32(len as i32), idx),
        );
        _mm256_movemask_ps(_mm256_castsi256_ps(ok)) == 0xff
    }
}

impl F32x8 {
    #[inline(always)]
    pub fn splat(v: f32) -> F32x8 {
        unsafe {
            F32x8(_mm256_set1_ps(v))
        }
    }
    #[inline(always)]
    pub fn from_array(a: [f32; 8]) -> F32x8 {
        unsafe {
            // SAFETY: reads exactly 8 f32 from a local array.
            F32x8(_mm256_loadu_ps(a.as_ptr()))
        }
    }
    #[inline(always)]
    pub fn to_array(self) -> [f32; 8] {
        unsafe {
            let mut a = [0.0; 8];
            // SAFETY: writes exactly 8 f32 into a local array.
            _mm256_storeu_ps(a.as_mut_ptr(), self.0);
            a
        }
    }
    /// [start, start + 1, ..., start + 7]
    #[inline(always)]
    pub fn ramp(start: f32) -> F32x8 {
        unsafe {
            F32x8(_mm256_add_ps(_mm256_set1_ps(start), _mm256_setr_ps(0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0)))
        }
    }
    #[inline(always)]
    pub fn load(s: &[f32]) -> F32x8 {
        unsafe {
            assert!(s.len() >= 8);
            // SAFETY: at least 8 readable elements.
            F32x8(_mm256_loadu_ps(s.as_ptr()))
        }
    }
    #[inline(always)]
    pub fn store(self, s: &mut [f32]) {
        unsafe {
            assert!(s.len() >= 8);
            // SAFETY: at least 8 writable elements.
            _mm256_storeu_ps(s.as_mut_ptr(), self.0)
        }
    }
    /// `base[idx[lane]]` per lane. Panics if any index is out of bounds.
    #[inline(always)]
    pub fn gather(base: &[f32], idx: I32x8) -> F32x8 {
        unsafe {
            assert!(in_bounds(idx.0, base.len()), "gather index out of bounds");
            // SAFETY: every index was checked against base.len().
            F32x8(_mm256_i32gather_ps::<4>(base.as_ptr(), idx.0))
        }
    }
    #[inline(always)]
    pub fn from_bits(i: I32x8) -> F32x8 {
        unsafe {
            F32x8(_mm256_castsi256_ps(i.0))
        }
    }
    /// self * a + b, fused (one rounding).
    #[inline(always)]
    pub fn mul_add(self, a: F32x8, b: F32x8) -> F32x8 {
        unsafe {
            F32x8(_mm256_fmadd_ps(self.0, a.0, b.0))
        }
    }
    #[inline(always)]
    pub fn min(self, o: F32x8) -> F32x8 {
        unsafe {
            F32x8(_mm256_min_ps(self.0, o.0))
        }
    }
    #[inline(always)]
    pub fn max(self, o: F32x8) -> F32x8 {
        unsafe {
            F32x8(_mm256_max_ps(self.0, o.0))
        }
    }
    #[inline(always)]
    pub fn sqrt(self) -> F32x8 {
        unsafe {
            F32x8(_mm256_sqrt_ps(self.0))
        }
    }
    #[inline(always)]
    pub fn floor(self) -> F32x8 {
        unsafe {
            F32x8(_mm256_floor_ps(self.0))
        }
    }
    /// Round half to even.
    #[inline(always)]
    pub fn round(self) -> F32x8 {
        unsafe {
            F32x8(_mm256_round_ps::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(self.0))
        }
    }
    /// Truncating conversion.
    #[inline(always)]
    pub fn to_i32(self) -> I32x8 {
        unsafe {
            I32x8(_mm256_cvttps_epi32(self.0))
        }
    }
    #[inline(always)]
    pub fn lt(self, o: F32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_cmp_ps::<_CMP_LT_OQ>(self.0, o.0))
        }
    }
    #[inline(always)]
    pub fn le(self, o: F32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_cmp_ps::<_CMP_LE_OQ>(self.0, o.0))
        }
    }
    #[inline(always)]
    pub fn gt(self, o: F32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_cmp_ps::<_CMP_GT_OQ>(self.0, o.0))
        }
    }
    #[inline(always)]
    pub fn ge(self, o: F32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_cmp_ps::<_CMP_GE_OQ>(self.0, o.0))
        }
    }
    /// Horizontal sum, lanes added in order 0..8 (same result as the portable version).
    #[inline(always)]
    pub fn sum(self) -> f32 {
        self.to_array().iter().sum()
    }
}

macro_rules! f32_op {
    ($tr:ident, $f:ident, $intr:ident) => {
        impl $tr for F32x8 {
            type Output = F32x8;
            #[inline(always)]
            fn $f(self, o: F32x8) -> F32x8 {
                F32x8(unsafe { $intr(self.0, o.0) })
            }
        }
    };
}
f32_op!(Add, add, _mm256_add_ps);
f32_op!(Sub, sub, _mm256_sub_ps);
f32_op!(Mul, mul, _mm256_mul_ps);
f32_op!(Div, div, _mm256_div_ps);

impl Neg for F32x8 {
    type Output = F32x8;
    #[inline(always)]
    fn neg(self) -> F32x8 {
        unsafe {
            F32x8(_mm256_xor_ps(self.0, _mm256_set1_ps(-0.0)))
        }
    }
}

impl I32x8 {
    #[inline(always)]
    pub fn splat(v: i32) -> I32x8 {
        unsafe {
            I32x8(_mm256_set1_epi32(v))
        }
    }
    #[inline(always)]
    pub fn from_array(a: [i32; 8]) -> I32x8 {
        unsafe {
            // SAFETY: reads 32 bytes from a local array.
            I32x8(_mm256_loadu_si256(a.as_ptr().cast()))
        }
    }
    #[inline(always)]
    pub fn to_array(self) -> [i32; 8] {
        unsafe {
            let mut a = [0; 8];
            // SAFETY: writes 32 bytes into a local array.
            _mm256_storeu_si256(a.as_mut_ptr().cast(), self.0);
            a
        }
    }
    #[inline(always)]
    pub fn ramp(start: i32) -> I32x8 {
        unsafe {
            I32x8(_mm256_add_epi32(_mm256_set1_epi32(start), _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7)))
        }
    }
    /// Loads 8 `u32` as their bit patterns.
    #[inline(always)]
    pub fn load_u32(s: &[u32]) -> I32x8 {
        unsafe {
            assert!(s.len() >= 8);
            // SAFETY: at least 8 readable elements.
            I32x8(_mm256_loadu_si256(s.as_ptr().cast()))
        }
    }
    #[inline(always)]
    pub fn store_u32(self, s: &mut [u32]) {
        unsafe {
            assert!(s.len() >= 8);
            // SAFETY: at least 8 writable elements.
            _mm256_storeu_si256(s.as_mut_ptr().cast(), self.0)
        }
    }
    /// `base[idx[lane]]` per lane, as bit patterns. Panics if any index is out of bounds.
    #[inline(always)]
    pub fn gather_u32(base: &[u32], idx: I32x8) -> I32x8 {
        unsafe {
            assert!(in_bounds(idx.0, base.len()), "gather index out of bounds");
            // SAFETY: every index was checked against base.len(); u32 and i32 share a layout.
            I32x8(_mm256_i32gather_epi32::<4>(base.as_ptr().cast(), idx.0))
        }
    }
    #[inline(always)]
    pub fn to_f32(self) -> F32x8 {
        unsafe {
            F32x8(_mm256_cvtepi32_ps(self.0))
        }
    }
    #[inline(always)]
    pub fn shl<const N: i32>(self) -> I32x8 {
        unsafe {
            I32x8(_mm256_slli_epi32::<N>(self.0))
        }
    }
    #[inline(always)]
    pub fn min(self, o: I32x8) -> I32x8 {
        unsafe {
            I32x8(_mm256_min_epi32(self.0, o.0))
        }
    }
    #[inline(always)]
    pub fn max(self, o: I32x8) -> I32x8 {
        unsafe {
            I32x8(_mm256_max_epi32(self.0, o.0))
        }
    }
    #[inline(always)]
    pub fn eq(self, o: I32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_castsi256_ps(_mm256_cmpeq_epi32(self.0, o.0)))
        }
    }
    #[inline(always)]
    pub fn gt(self, o: I32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_castsi256_ps(_mm256_cmpgt_epi32(self.0, o.0)))
        }
    }
}

macro_rules! i32_op {
    ($tr:ident, $f:ident, $intr:ident) => {
        impl $tr for I32x8 {
            type Output = I32x8;
            #[inline(always)]
            fn $f(self, o: I32x8) -> I32x8 {
                I32x8(unsafe { $intr(self.0, o.0) })
            }
        }
    };
}
i32_op!(Add, add, _mm256_add_epi32);
i32_op!(Sub, sub, _mm256_sub_epi32);
i32_op!(Mul, mul, _mm256_mullo_epi32);
i32_op!(BitOr, bitor, _mm256_or_si256);

impl M32x8 {
    /// Lane i set iff bit i of `bits` is set.
    #[inline(always)]
    pub fn from_bits(bits: u32) -> M32x8 {
        unsafe {
            let lane_bits = _mm256_setr_epi32(1, 2, 4, 8, 16, 32, 64, 128);
            let set = _mm256_and_si256(_mm256_set1_epi32(bits as i32), lane_bits);
            M32x8(_mm256_castsi256_ps(_mm256_cmpeq_epi32(set, lane_bits)))
        }
    }
    /// Bit i set iff lane i is set.
    #[inline(always)]
    pub fn bits(self) -> u32 {
        unsafe {
            _mm256_movemask_ps(self.0) as u32
        }
    }
    #[inline(always)]
    pub fn any(self) -> bool {
        self.bits() != 0
    }
    #[inline(always)]
    pub fn all(self) -> bool {
        self.bits() == 0xff
    }
    #[inline(always)]
    pub fn count(self) -> u32 {
        self.bits().count_ones()
    }
    /// `a` where the lane is set, `b` elsewhere.
    #[inline(always)]
    pub fn select(self, a: F32x8, b: F32x8) -> F32x8 {
        unsafe {
            F32x8(_mm256_blendv_ps(b.0, a.0, self.0))
        }
    }
    #[inline(always)]
    pub fn select_i32(self, a: I32x8, b: I32x8) -> I32x8 {
        unsafe {
            I32x8(_mm256_castps_si256(_mm256_blendv_ps(_mm256_castsi256_ps(b.0), _mm256_castsi256_ps(a.0), self.0)))
        }
    }
}

impl BitAnd for M32x8 {
    type Output = M32x8;
    #[inline(always)]
    fn bitand(self, o: M32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_and_ps(self.0, o.0))
        }
    }
}

impl BitOr for M32x8 {
    type Output = M32x8;
    #[inline(always)]
    fn bitor(self, o: M32x8) -> M32x8 {
        unsafe {
            M32x8(_mm256_or_ps(self.0, o.0))
        }
    }
}

impl Not for M32x8 {
    type Output = M32x8;
    #[inline(always)]
    fn not(self) -> M32x8 {
        unsafe {
            M32x8(_mm256_xor_ps(self.0, _mm256_castsi256_ps(_mm256_set1_epi32(-1))))
        }
    }
}
