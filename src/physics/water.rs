//! Buoyancy and drag inside an axis-aligned water volume (Archimedes: the push up equals the weight
//! of the displaced water). Built only on `PhysicsWorld`'s public data, like a game system would be.

use crate::{
    jobs::Jobs,
    math::{Vec3, vec3},
    physics::PhysicsWorld,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterVolume {
    /// Bottom corner of the volume; `max.y` is the surface.
    pub min: Vec3,
    pub max: Vec3,
    /// kg/m³. Bodies lighter than this float.
    pub density: f32,
    /// Velocity damping per second while fully submerged.
    pub drag: f32,
}

impl WaterVolume {
    pub fn surface(&self) -> f32 {
        self.max.y
    }

    fn contains_xz(&self, p: Vec3) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.z >= self.min.z && p.z <= self.max.z
    }

    /// One tick of buoyancy and drag: velocity changes for particles (data-parallel over all of
    /// them), forces at 27 sample points for rigid bodies, so a tilted box gets a righting torque.
    pub fn apply(&self, phys: &mut PhysicsWorld, jobs: &Jobs, dt: f32) {
        let g = -phys.gravity.y;
        let water = *self;
        let surface = self.surface();
        jobs.par_zip((&mut phys.vel, &phys.pos, &phys.radius, &phys.inv_mass), 1024, |_, (vel, pos, radius, w)| {
            for i in 0..vel.len() {
                let (p, r) = (pos[i], radius[i]);
                if w[i] == 0.0 || !water.contains_xz(p) || p.y - r > surface || p.y + r < water.min.y {
                    continue;
                }
                let frac = submerged((surface - (p.y - r)) / (2.0 * r));
                let volume = 4.0 / 3.0 * std::f32::consts::PI * r * r * r;
                let lift = water.density * volume * frac * g * w[i];
                vel[i] += (Vec3::Y * lift - vel[i] * (water.drag * frac)) * dt;
            }
        });

        for body in &mut phys.rigids {
            if !body.is_dynamic() || !self.contains_xz(body.pos) {
                continue;
            }
            let h = body.half;
            let cell_volume = 8.0 * h.x * h.y * h.z / 27.0;
            let cell_half = (h.x + h.y + h.z) / 9.0;
            let cell_mass = body.mass() / 27.0;
            for x in [-1.0, 0.0, 1.0] {
                for y in [-1.0, 0.0, 1.0] {
                    for z in [-1.0, 0.0, 1.0] {
                        let p = body.to_world(vec3(x, y, z) * h * (2.0 / 3.0));
                        let frac = submerged((surface - (p.y - cell_half)) / (2.0 * cell_half));
                        if frac > 0.0 {
                            let lift = Vec3::Y * (self.density * cell_volume * frac * g);
                            let drag = body.point_velocity(p) * (-self.drag * cell_mass * frac);
                            body.add_force_at(lift + drag, p);
                        }
                    }
                }
            }
        }
    }
}

/// Submerged volume fraction of a sphere whose depth is `x` diameters (0 = touching the surface
/// from above, 1 = fully under): the spherical cap volume, `x^2 (3 - 2x)`.
fn submerged(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{jobs::JobSystem, physics::RigidBody};

    #[test]
    fn light_things_float_heavy_things_sink() {
        let jobs = JobSystem::new(1);
        let mut w = PhysicsWorld::default();
        let water = WaterVolume { min: vec3(-10.0, 0.0, -10.0), max: vec3(10.0, 3.0, 10.0), density: 1000.0, drag: 2.0 };
        let volume = |r: f32| 4.0 / 3.0 * std::f32::consts::PI * r * r * r;
        let cork = w.add_body(vec3(-2.0, 1.0, 0.0), 0.3, 500.0 * volume(0.3));
        let stone = w.add_body(vec3(2.0, 2.0, 0.0), 0.3, 3000.0 * volume(0.3));
        let crate_ = w.add_rigid(RigidBody::new_box(vec3(0.0, 1.0, 4.0), Vec3::splat(0.5), 400.0));
        // The sandbox car's chassis: 1200 kg in 4.3 m³.
        let chassis = w.add_rigid(RigidBody::new_box(vec3(4.0, 1.0, -4.0), vec3(0.9, 0.3, 2.0), 1200.0));
        for _ in 0..600 {
            water.apply(&mut w, &jobs, 1.0 / 60.0);
            w.step(&jobs, 1.0 / 60.0);
        }
        let cork_y = w.pos[cork.0 as usize].y;
        assert!((cork_y - 3.0).abs() < 0.05, "half-density ball floats with its center at the surface: {cork_y}");
        assert!(w.pos[stone.0 as usize].y < 0.31, "stone rests on the bottom");
        let crate_y = w.rigid(crate_).pos.y;
        assert!(crate_y > 2.5 && crate_y < 3.5, "crate floats near the surface: {crate_y}");
        let car = w.rigid(chassis);
        assert!(car.pos.y > 2.8 && car.rot.rotate(Vec3::Y).y > 0.95, "chassis floats flat: {:?}", car.pos);
    }
}
