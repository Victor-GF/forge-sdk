//! Box geometry shared by the solver and the queries. Everything works in the box's local frame,
//! where an oriented box is just `-half..half`.

use crate::math::{Quat, Vec3};

/// Box that never moves: ramps, walls, platforms. Cheap to add; particles, rigid bodies and rays
/// all collide with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticBox {
    pub center: Vec3,
    pub rot: Quat,
    pub half: Vec3,
}

impl StaticBox {
    pub fn new(center: Vec3, half: Vec3) -> StaticBox {
        StaticBox { center, rot: Quat::IDENTITY, half }
    }
    pub fn with_rot(mut self, rot: Quat) -> StaticBox {
        self.rot = rot;
        self
    }
    pub(crate) fn obb(&self) -> Obb {
        Obb { center: self.center, rot: self.rot, half: self.half }
    }
}

/// Oriented box used by the tests below (static boxes and rigid bodies both turn into one).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Obb {
    pub center: Vec3,
    pub rot: Quat,
    pub half: Vec3,
}

impl Obb {
    pub fn bound_radius(&self) -> f32 {
        self.half.length()
    }

    /// Sphere at `p` with radius `r` against the box. Returns the normal pointing out of the box
    /// towards the sphere and the penetration depth, or `None` when they don't touch.
    #[inline]
    pub fn sphere_contact(&self, p: Vec3, r: f32) -> Option<(Vec3, f32)> {
        let q = self.rot.unrotate(p - self.center);
        let clamped = q.max(-self.half).min(self.half);
        let d = q - clamped;
        let d2 = d.length_sq();
        if d2 > 0.0 {
            if d2 >= r * r {
                return None;
            }
            let dist = d2.sqrt();
            return Some((self.rot.rotate(d / dist), r - dist));
        }
        // Center inside the box: leave through the nearest face.
        let gap = self.half - q.abs();
        let (q, gap) = (q.to_array(), gap.to_array());
        let axis = if gap[0] <= gap[1] && gap[0] <= gap[2] {
            0
        } else if gap[1] <= gap[2] {
            1
        } else {
            2
        };
        let mut n = [0.0; 3];
        n[axis] = if q[axis] < 0.0 { -1.0 } else { 1.0 };
        Some((self.rot.rotate(Vec3::from_array(n)), gap[axis] + r))
    }

    /// Ray against the box (slab test). `dir` must be unit length. Rays starting inside miss.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<(f32, Vec3)> {
        let o = self.rot.unrotate(origin - self.center).to_array();
        let d = self.rot.unrotate(dir).to_array();
        let h = self.half.to_array();
        let (mut t_in, mut t_out, mut axis, mut sign) = (f32::NEG_INFINITY, max_t, 0, 0.0);
        for i in 0..3 {
            if d[i].abs() < 1e-12 {
                if o[i].abs() > h[i] {
                    return None;
                }
                continue;
            }
            let inv = 1.0 / d[i];
            let (mut a, mut b) = ((-h[i] - o[i]) * inv, (h[i] - o[i]) * inv);
            let mut s = -1.0;
            if a > b {
                std::mem::swap(&mut a, &mut b);
                s = 1.0;
            }
            if a > t_in {
                (t_in, axis, sign) = (a, i, s);
            }
            t_out = t_out.min(b);
            if t_in > t_out {
                return None;
            }
        }
        if t_in < 0.0 {
            return None;
        }
        let mut n = [0.0; 3];
        n[axis] = sign;
        Some((t_in, self.rot.rotate(Vec3::from_array(n))))
    }
}

/// Ray against a sphere; `dir` must be unit length. Rays starting inside miss.
pub(crate) fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, r: f32, max_t: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let c = oc.length_sq() - r * r;
    let disc = b * b - c;
    if c <= 0.0 || disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t >= 0.0 && t <= max_t).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::vec3;

    #[test]
    fn sphere_contact_outside_and_inside() {
        let b = Obb { center: Vec3::ZERO, rot: Quat::IDENTITY, half: Vec3::ONE };
        let (n, depth) = b.sphere_contact(vec3(0.0, 1.2, 0.0), 0.5).unwrap();
        assert!((n - Vec3::Y).length() < 1e-6 && (depth - 0.3).abs() < 1e-6);
        assert!(b.sphere_contact(vec3(0.0, 1.6, 0.0), 0.5).is_none());
        let (n, depth) = b.sphere_contact(vec3(0.9, 0.0, 0.2), 0.1).unwrap();
        assert!((n - Vec3::X).length() < 1e-6 && (depth - 0.2).abs() < 1e-5);
    }

    #[test]
    fn ray_hits_rotated_box() {
        let rot = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_4);
        let b = Obb { center: vec3(0.0, 0.0, -5.0), rot, half: Vec3::ONE };
        let (t, n) = b.ray(Vec3::ZERO, -Vec3::Z, 100.0).unwrap();
        // The corner of a 45-degree box points at the ray: distance 5 - sqrt(2).
        assert!((t - (5.0 - 2f32.sqrt())).abs() < 1e-4, "t = {t}");
        assert!(n.z > 0.0);
        assert!(b.ray(Vec3::ZERO, Vec3::Z, 100.0).is_none());
        assert_eq!(ray_sphere(Vec3::ZERO, Vec3::X, vec3(3.0, 0.0, 0.0), 1.0, 10.0), Some(2.0));
    }
}
