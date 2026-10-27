//! Portable 8-lane implementation: plain arrays and loops (LLVM vectorizes most of them for the
//! baseline target, e.g. SSE2 on x86-64 or NEON on aarch64).

use std::ops::{Add, BitAnd, BitOr, Div, Mul, Neg, Not, Sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct F32x8(pub(crate) [f32; 8]);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct I32x8(pub(crate) [i32; 8]);
/// Lane mask: one bool per lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct M32x8(pub(crate) [bool; 8]);

macro_rules! map {
    ($a:expr, |$x:ident| $body:expr) => {{
        let a = $a;
        std::array::from_fn(|i| {
            let $x = a[i];
            $body
        })
    }};
    ($a:expr, $b:expr, |$x:ident, $y:ident| $body:expr) => {{
        let (a, b) = ($a, $b);
        std::array::from_fn(|i| {
            let ($x, $y) = (a[i], b[i]);
            $body
        })
    }};
}

fn in_bounds(idx: [i32; 8], len: usize) -> bool {
    idx.iter().all(|&i| i >= 0 && (i as usize) < len)
}

impl F32x8 {
    #[inline(always)]
    pub fn splat(v: f32) -> F32x8 {
        F32x8([v; 8])
    }
    #[inline(always)]
    pub fn from_array(a: [f32; 8]) -> F32x8 {
        F32x8(a)
    }
    #[inline(always)]
    pub fn to_array(self) -> [f32; 8] {
        self.0
    }
    /// [start, start + 1, ..., start + 7]
    #[inline(always)]
    pub fn ramp(start: f32) -> F32x8 {
        F32x8(std::array::from_fn(|i| start + i as f32))
    }
    #[inline(always)]
    pub fn load(s: &[f32]) -> F32x8 {
        F32x8(s[..8].try_into().unwrap())
    }
    #[inline(always)]
    pub fn store(self, s: &mut [f32]) {
        s[..8].copy_from_slice(&self.0);
    }
    /// `base[idx[lane]]` per lane. Panics if any index is out of bounds.
    #[inline(always)]
    pub fn gather(base: &[f32], idx: I32x8) -> F32x8 {
        assert!(in_bounds(idx.0, base.len()), "gather index out of bounds");
        F32x8(map!(idx.0, |i| base[i as usize]))
    }
    #[inline(always)]
    pub fn from_bits(i: I32x8) -> F32x8 {
        F32x8(map!(i.0, |x| f32::from_bits(x as u32)))
    }
    #[inline(always)]
    pub fn mul_add(self, a: F32x8, b: F32x8) -> F32x8 {
        F32x8(std::array::from_fn(|i| self.0[i] * a.0[i] + b.0[i]))
    }
    #[inline(always)]
    pub fn min(self, o: F32x8) -> F32x8 {
        F32x8(map!(self.0, o.0, |a, b| if b < a { b } else { a }))
    }
    #[inline(always)]
    pub fn max(self, o: F32x8) -> F32x8 {
        F32x8(map!(self.0, o.0, |a, b| if b > a { b } else { a }))
    }
    #[inline(always)]
    pub fn sqrt(self) -> F32x8 {
        F32x8(map!(self.0, |a| a.sqrt()))
    }
    #[inline(always)]
    pub fn floor(self) -> F32x8 {
        F32x8(map!(self.0, |a| a.floor()))
    }
    /// Round half to even, like the hardware.
    #[inline(always)]
    pub fn round(self) -> F32x8 {
        F32x8(map!(self.0, |a| a.round_ties_even()))
    }
    /// Truncating conversion.
    #[inline(always)]
    pub fn to_i32(self) -> I32x8 {
        I32x8(map!(self.0, |a| a as i32))
    }
    #[inline(always)]
    pub fn lt(self, o: F32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a < b))
    }
    #[inline(always)]
    pub fn le(self, o: F32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a <= b))
    }
    #[inline(always)]
    pub fn gt(self, o: F32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a > b))
    }
    #[inline(always)]
    pub fn ge(self, o: F32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a >= b))
    }
    /// Horizontal sum, lanes added in order 0..8.
    #[inline(always)]
    pub fn sum(self) -> f32 {
        self.0.iter().sum()
    }
}

macro_rules! f32_op {
    ($tr:ident, $f:ident, $op:tt) => {
        impl $tr for F32x8 {
            type Output = F32x8;
            #[inline(always)]
            fn $f(self, o: F32x8) -> F32x8 {
                F32x8(map!(self.0, o.0, |a, b| a $op b))
            }
        }
    };
}
f32_op!(Add, add, +);
f32_op!(Sub, sub, -);
f32_op!(Mul, mul, *);
f32_op!(Div, div, /);

impl Neg for F32x8 {
    type Output = F32x8;
    #[inline(always)]
    fn neg(self) -> F32x8 {
        F32x8(map!(self.0, |a| -a))
    }
}

impl I32x8 {
    #[inline(always)]
    pub fn splat(v: i32) -> I32x8 {
        I32x8([v; 8])
    }
    #[inline(always)]
    pub fn from_array(a: [i32; 8]) -> I32x8 {
        I32x8(a)
    }
    #[inline(always)]
    pub fn to_array(self) -> [i32; 8] {
        self.0
    }
    #[inline(always)]
    pub fn ramp(start: i32) -> I32x8 {
        I32x8(std::array::from_fn(|i| start + i as i32))
    }
    /// Loads 8 `u32` as their bit patterns.
    #[inline(always)]
    pub fn load_u32(s: &[u32]) -> I32x8 {
        I32x8(std::array::from_fn(|i| s[..8][i] as i32))
    }
    #[inline(always)]
    pub fn store_u32(self, s: &mut [u32]) {
        for (d, v) in s[..8].iter_mut().zip(self.0) {
            *d = v as u32;
        }
    }
    /// `base[idx[lane]]` per lane, as bit patterns. Panics if any index is out of bounds.
    #[inline(always)]
    pub fn gather_u32(base: &[u32], idx: I32x8) -> I32x8 {
        assert!(in_bounds(idx.0, base.len()), "gather index out of bounds");
        I32x8(map!(idx.0, |i| base[i as usize] as i32))
    }
    #[inline(always)]
    pub fn to_f32(self) -> F32x8 {
        F32x8(map!(self.0, |a| a as f32))
    }
    #[inline(always)]
    pub fn shl<const N: i32>(self) -> I32x8 {
        I32x8(map!(self.0, |a| a << N))
    }
    #[inline(always)]
    pub fn min(self, o: I32x8) -> I32x8 {
        I32x8(map!(self.0, o.0, |a, b| a.min(b)))
    }
    #[inline(always)]
    pub fn max(self, o: I32x8) -> I32x8 {
        I32x8(map!(self.0, o.0, |a, b| a.max(b)))
    }
    #[inline(always)]
    pub fn eq(self, o: I32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a == b))
    }
    #[inline(always)]
    pub fn gt(self, o: I32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a > b))
    }
}

macro_rules! i32_op {
    ($tr:ident, $f:ident, $op:ident) => {
        impl $tr for I32x8 {
            type Output = I32x8;
            #[inline(always)]
            fn $f(self, o: I32x8) -> I32x8 {
                I32x8(map!(self.0, o.0, |a, b| a.$op(b)))
            }
        }
    };
}
i32_op!(Add, add, wrapping_add);
i32_op!(Sub, sub, wrapping_sub);
i32_op!(Mul, mul, wrapping_mul);

impl BitOr for I32x8 {
    type Output = I32x8;
    #[inline(always)]
    fn bitor(self, o: I32x8) -> I32x8 {
        I32x8(map!(self.0, o.0, |a, b| a | b))
    }
}

impl M32x8 {
    /// Lane i set iff bit i of `bits` is set.
    #[inline(always)]
    pub fn from_bits(bits: u32) -> M32x8 {
        M32x8(std::array::from_fn(|i| bits >> i & 1 != 0))
    }
    /// Bit i set iff lane i is set.
    #[inline(always)]
    pub fn bits(self) -> u32 {
        self.0.iter().enumerate().map(|(i, &b)| (b as u32) << i).sum()
    }
    #[inline(always)]
    pub fn any(self) -> bool {
        self.0.iter().any(|&b| b)
    }
    #[inline(always)]
    pub fn all(self) -> bool {
        self.0.iter().all(|&b| b)
    }
    #[inline(always)]
    pub fn count(self) -> u32 {
        self.bits().count_ones()
    }
    /// `a` where the lane is set, `b` elsewhere.
    #[inline(always)]
    pub fn select(self, a: F32x8, b: F32x8) -> F32x8 {
        F32x8(std::array::from_fn(|i| if self.0[i] { a.0[i] } else { b.0[i] }))
    }
    #[inline(always)]
    pub fn select_i32(self, a: I32x8, b: I32x8) -> I32x8 {
        I32x8(std::array::from_fn(|i| if self.0[i] { a.0[i] } else { b.0[i] }))
    }
}

impl BitAnd for M32x8 {
    type Output = M32x8;
    #[inline(always)]
    fn bitand(self, o: M32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a & b))
    }
}

impl BitOr for M32x8 {
    type Output = M32x8;
    #[inline(always)]
    fn bitor(self, o: M32x8) -> M32x8 {
        M32x8(map!(self.0, o.0, |a, b| a | b))
    }
}

impl Not for M32x8 {
    type Output = M32x8;
    #[inline(always)]
    fn not(self) -> M32x8 {
        M32x8(map!(self.0, |a| !a))
    }
}
