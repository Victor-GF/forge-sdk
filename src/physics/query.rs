//! Queries game systems run against the physics world: raycasts and radial impulses.
//!
//! Particles are tested by brute force, 8 at a time with SIMD. A ray against 3000 spheres is
//! ~400 `F32x8` iterations, a few microseconds, with no acceleration structure to keep up to date.
//! A game firing hundreds of rays per tick would want a BVH or a DDA walk over the hash grid.

use crate::{
    math::Vec3,
    physics::{BodyId, PhysicsWorld, RigidId, shapes::ray_sphere, vec3_words},
    simd::{F32x8, I32x8},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    /// The floor or a wall of the world box.
    Bounds,
    Static(usize),
    Particle(BodyId),
    Rigid(RigidId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub target: HitTarget,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RayFilter {
    /// Skip this rigid body (a vehicle's wheels must not hit its own chassis).
    pub ignore_rigid: Option<RigidId>,
    pub skip_particles: bool,
}

impl RayFilter {
    pub fn ignoring(rigid: RigidId) -> RayFilter {
        RayFilter { ignore_rigid: Some(rigid), skip_particles: false }
    }
}

impl PhysicsWorld {
    /// Closest hit along `origin + dir * t` for `t` in `0..=max_t`. `dir` must be unit length.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_t: f32, filter: RayFilter) -> Option<RayHit> {
        let mut best: Option<(f32, Vec3, HitTarget)> = None;
        let mut limit = max_t;
        let mut consider = |t: f32, n: Vec3, target: HitTarget, limit: &mut f32| {
            if t <= *limit {
                *limit = t;
                best = Some((t, n, target));
            }
        };

        // Inside of the world box: the ray leaves through the nearest wall it is heading to.
        let (o, d) = (origin.to_array(), dir.to_array());
        let (lo, hi) = (self.bounds_min.to_array(), self.bounds_max.to_array());
        for k in 0..3 {
            let wall = if d[k] < 0.0 { lo[k] } else { hi[k] };
            if d[k] != 0.0 && wall.is_finite() {
                let t = (wall - o[k]) / d[k];
                if t >= 0.0 {
                    let mut n = [0.0; 3];
                    n[k] = -d[k].signum();
                    consider(t, Vec3::from_array(n), HitTarget::Bounds, &mut limit);
                }
            }
        }
        for (k, s) in self.statics.iter().enumerate() {
            if let Some((t, n)) = s.obb().ray(origin, dir, limit) {
                consider(t, n, HitTarget::Static(k), &mut limit);
            }
        }
        for (k, body) in self.rigids.iter().enumerate() {
            let id = RigidId(k as u32);
            if filter.ignore_rigid == Some(id) {
                continue;
            }
            if let Some((t, n)) = body.obb().ray(origin, dir, limit) {
                consider(t, n, HitTarget::Rigid(id), &mut limit);
            }
        }
        if !filter.skip_particles
            && let Some((t, i)) = self.raycast_particles(origin, dir, limit)
        {
            let n = (origin + dir * t - self.pos[i]) / self.radius[i];
            consider(t, n, HitTarget::Particle(BodyId(i as u32)), &mut limit);
        }
        best.map(|(t, normal, target)| RayHit { t, point: origin + dir * t, normal, target })
    }

    /// Nearest particle hit, 8 spheres per iteration.
    fn raycast_particles(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<(f32, usize)> {
        let n = self.pos.len();
        if n == 0 {
            return None;
        }
        let words = vec3_words(&self.pos);
        let (ox, oy, oz) = (F32x8::splat(origin.x), F32x8::splat(origin.y), F32x8::splat(origin.z));
        let (dx, dy, dz) = (F32x8::splat(dir.x), F32x8::splat(dir.y), F32x8::splat(dir.z));
        let zero = F32x8::splat(0.0);
        let mut best_t = F32x8::splat(max_t);
        let mut best_i = I32x8::splat(-1);
        for base in (0..n).step_by(8) {
            let idx = I32x8::ramp(base as i32);
            let valid = I32x8::splat(n as i32).gt(idx);
            let idx = valid.select_i32(idx, I32x8::splat(0)); // keep gathers in bounds
            let p3 = idx * I32x8::splat(3);
            let ocx = ox - F32x8::gather(words, p3);
            let ocy = oy - F32x8::gather(words, p3 + I32x8::splat(1));
            let ocz = oz - F32x8::gather(words, p3 + I32x8::splat(2));
            let r = F32x8::gather(&self.radius, idx);
            let b = ocx.mul_add(dx, ocy.mul_add(dy, ocz * dz));
            let c = ocx.mul_add(ocx, ocy.mul_add(ocy, ocz * ocz)) - r * r;
            let disc = b.mul_add(b, -c);
            let t = -b - disc.max(zero).sqrt();
            let hit = valid & c.gt(zero) & disc.ge(zero) & t.ge(zero) & t.lt(best_t);
            if hit.any() {
                best_t = hit.select(t, best_t);
                best_i = hit.select_i32(idx, best_i);
            }
        }
        let (ts, is) = (best_t.to_array(), best_i.to_array());
        let mut best: Option<(f32, usize)> = None;
        for k in 0..8 {
            if is[k] >= 0 {
                let cand = (ts[k], is[k] as usize);
                // Ties go to the lower index so the answer doesn't depend on lane layout.
                if best.is_none_or(|b| cand.0 < b.0 || (cand.0 == b.0 && cand.1 < b.1)) {
                    best = Some(cand);
                }
            }
        }
        // Lanes only race each other, so confirm with the scalar test (also catches FMA rounding).
        best.filter(|&(_, i)| ray_sphere(origin, dir, self.pos[i], self.radius[i], max_t).is_some())
    }

    /// Radial impulse with linear falloff: `strength` at the center, 0 at `radius`. Light bodies
    /// fly further than heavy ones (it's an impulse, not a velocity).
    pub fn explode(&mut self, center: Vec3, radius: f32, strength: f32) {
        for i in 0..self.pos.len() {
            let d = self.pos[i] - center;
            let dist = d.length();
            if dist < radius && dist > 1e-6 && self.inv_mass[i] > 0.0 {
                self.vel[i] += d / dist * (strength * (1.0 - dist / radius) * self.inv_mass[i]);
            }
        }
        for body in &mut self.rigids {
            // Push at the box point nearest the blast, so off-center blasts spin it.
            let local = body.rot.unrotate(center - body.pos);
            let near = body.to_world(local.max(-body.half).min(body.half));
            let d = body.pos - center;
            let dist = (near - center).length();
            if dist < radius {
                body.apply_impulse_at(d.normalize() * (strength * (1.0 - dist / radius)), near);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        math::vec3,
        physics::{RigidBody, StaticBox},
    };

    #[test]
    fn raycast_returns_the_nearest_hit() {
        let mut w = PhysicsWorld::default();
        w.bounds_min = vec3(-50.0, 0.0, -50.0);
        w.bounds_max = vec3(50.0, f32::INFINITY, 50.0);
        for i in 0..21 {
            w.add_body(vec3(i as f32, 1.0, -10.0), 0.4, 1.0); // a row the ray passes under
        }
        let ball = w.add_body(vec3(0.0, 1.0, -6.0), 0.5, 1.0);
        w.add_static(StaticBox::new(vec3(0.0, 1.0, -8.0), vec3(1.0, 1.0, 0.2)));
        let crate_ = w.add_rigid(RigidBody::new_box(vec3(0.0, 1.0, -4.0), Vec3::splat(0.5), 10.0));

        let o = vec3(0.0, 1.0, 0.0);
        let hit = w.raycast(o, -Vec3::Z, 100.0, RayFilter::default()).unwrap();
        assert_eq!(hit.target, HitTarget::Rigid(crate_));
        assert!((hit.t - 3.5).abs() < 1e-5 && (hit.normal - Vec3::Z).length() < 1e-5);

        let hit = w.raycast(o, -Vec3::Z, 100.0, RayFilter::ignoring(crate_)).unwrap();
        assert_eq!(hit.target, HitTarget::Particle(ball));
        assert!((hit.t - 5.5).abs() < 1e-4);

        let filter = RayFilter { ignore_rigid: Some(crate_), skip_particles: true };
        let hit = w.raycast(o, -Vec3::Z, 100.0, filter).unwrap();
        assert_eq!(hit.target, HitTarget::Static(0));
        assert!((hit.t - 7.8).abs() < 1e-4);

        let hit = w.raycast(vec3(0.0, 3.0, 0.0), -Vec3::Y, 100.0, RayFilter::default()).unwrap();
        assert_eq!(hit.target, HitTarget::Bounds);
        assert!((hit.t - 3.0).abs() < 1e-6 && hit.normal == Vec3::Y);
        assert!(w.raycast(vec3(0.0, 3.0, 0.0), -Vec3::Y, 2.0, RayFilter::default()).is_none());
    }
}
