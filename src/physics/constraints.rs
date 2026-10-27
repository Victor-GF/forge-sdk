//! Distance constraints between particles: ropes, chains, nets, cloth.
//!
//! Solved Gauss-Seidel style, one constraint after another, on a single thread. A few thousand
//! constraints cost tens of microseconds, and a fixed order keeps the result deterministic. If this
//! ever shows up in a profile, the classic next step is graph coloring: constraints of one color
//! share no particle, so each color is a `par_for`.

use crate::math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistanceConstraint {
    pub a: u32,
    pub b: u32,
    pub rest: f32,
    /// Inverse stiffness (XPBD compliance, m/N). 0 = rigid rod.
    pub compliance: f32,
    /// Only pulls when longer than `rest`, never pushes: a tether. Used for long-range
    /// attachments (Kim et al. 2012), which stop long chains from stretching under load.
    pub tether: bool,
}

impl DistanceConstraint {
    pub fn rod(a: u32, b: u32, rest: f32) -> DistanceConstraint {
        DistanceConstraint { a, b, rest, compliance: 0.0, tether: false }
    }
    pub fn spring(a: u32, b: u32, rest: f32, compliance: f32) -> DistanceConstraint {
        DistanceConstraint { a, b, rest, compliance, tether: false }
    }
    pub fn tether(a: u32, b: u32, max: f32) -> DistanceConstraint {
        DistanceConstraint { a, b, rest: max, compliance: 0.0, tether: true }
    }
}

pub(crate) fn solve(constraints: &[DistanceConstraint], pos: &mut [Vec3], inv_mass: &[f32], h: f32) {
    let inv_h2 = 1.0 / (h * h);
    for c in constraints {
        let (a, b) = (c.a as usize, c.b as usize);
        let w = inv_mass[a] + inv_mass[b];
        if w == 0.0 {
            continue;
        }
        let d = pos[a] - pos[b];
        let len = d.length();
        let err = len - c.rest;
        if len < 1e-9 || (c.tether && err <= 0.0) {
            continue;
        }
        let n = d / len;
        let lambda = -err / (w + c.compliance * inv_h2);
        pos[a] += n * (lambda * inv_mass[a]);
        pos[b] -= n * (lambda * inv_mass[b]);
    }
}
