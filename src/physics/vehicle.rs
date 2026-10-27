//! Raycast vehicle: the car model of most action games (Bullet's `btRaycastVehicle`, Unity's
//! WheelCollider and countless arcade racers). The chassis is one rigid box; each wheel is a ray
//! cast down from its mount point. A ray that hits the ground becomes a spring-damper (the
//! suspension) plus tire forces at the contact point, all pushed into the chassis as forces. Wheels
//! have no mass and no collision shape of their own.
//!
//! It runs once per tick, before `physics.step`, through the same API any game system uses
//! (`raycast`, `add_force_at`, impulses). The reaction to every wheel force is applied to whatever
//! the wheel stands on, so driving over balls pushes them into the floor.

use crate::{
    math::{Quat, Vec3},
    physics::{HitTarget, PhysicsWorld, RayFilter, RigidId},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wheel {
    /// Top of the suspension, in chassis space.
    pub mount: Vec3,
    pub radius: f32,
    pub steers: bool,
    pub driven: bool,
    // State, updated every tick.
    /// How far the suspension is compressed, 0..=rest.
    pub compression: f32,
    /// Normal force on the ground (N); 0 when airborne.
    pub load: f32,
    /// Rolling angle, for drawing.
    pub spin: f32,
    pub contact: Option<(Vec3, Vec3)>,
}

impl Wheel {
    pub fn new(mount: Vec3, radius: f32, steers: bool, driven: bool) -> Wheel {
        Wheel { mount, radius, steers, driven, compression: 0.0, load: 0.0, spin: 0.0, contact: None }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleInput {
    /// -1 (reverse) ..= 1 (full throttle).
    pub throttle: f32,
    /// -1 (left) ..= 1 (right).
    pub steer: f32,
    /// 0 ..= 1.
    pub brake: f32,
}

#[derive(Clone, Debug)]
pub struct RaycastVehicle {
    pub body: RigidId,
    pub wheels: Vec<Wheel>,
    /// Suspension travel (m).
    pub rest_length: f32,
    /// N/m.
    pub stiffness: f32,
    /// N/(m/s).
    pub damping: f32,
    /// Tire friction coefficient.
    pub grip: f32,
    /// Total drive force (N), split between driven wheels.
    pub engine_force: f32,
    pub brake_force: f32,
    /// Maximum steering angle (rad) at standstill; it shrinks with speed.
    pub max_steer: f32,
    /// 0..=1: height at which tire forces act, from the center of mass (0, no body roll) to the
    /// contact point (1, realistic and easy to flip). The classic arcade-handling knob.
    pub roll_influence: f32,
    /// Air drag coefficient: force = -c * v * |v|.
    pub air_drag: f32,
    pub steer_angle: f32,
}

impl RaycastVehicle {
    /// Four wheels at the corners of a chassis of half size `half`: front wheels steer, rear
    /// wheels drive. Chassis forward is -Z.
    pub fn four_wheels(body: RigidId, half: Vec3, wheel_radius: f32) -> RaycastVehicle {
        let (x, y, z) = (half.x - 0.1, -half.y * 0.5, half.z - wheel_radius - 0.1);
        let wheels = vec![
            Wheel::new(Vec3 { x: -x, y, z: -z }, wheel_radius, true, false),
            Wheel::new(Vec3 { x, y, z: -z }, wheel_radius, true, false),
            Wheel::new(Vec3 { x: -x, y, z }, wheel_radius, false, true),
            Wheel::new(Vec3 { x, y, z }, wheel_radius, false, true),
        ];
        RaycastVehicle {
            body,
            wheels,
            rest_length: 0.45,
            stiffness: 30_000.0,
            damping: 3_500.0,
            grip: 1.1,
            engine_force: 7_000.0,
            brake_force: 9_000.0,
            max_steer: 0.6,
            roll_influence: 0.25,
            air_drag: 0.45,
            steer_angle: 0.0,
        }
    }

    /// Signed speed along the chassis' forward axis (m/s).
    pub fn forward_speed(&self, phys: &PhysicsWorld) -> f32 {
        let body = phys.rigid(self.body);
        body.vel.dot(body.rot.rotate(-Vec3::Z))
    }

    pub fn update(&mut self, phys: &mut PhysicsWorld, input: VehicleInput, dt: f32) {
        let speed = self.forward_speed(phys);
        let target = input.steer.clamp(-1.0, 1.0) * self.max_steer / (1.0 + speed.abs() / 12.0);
        let step = 2.5 * dt;
        self.steer_angle += (target - self.steer_angle).clamp(-step, step);

        let body = phys.rigid(self.body).clone();
        let up = body.rot.rotate(Vec3::Y);
        let wheel_count = self.wheels.len() as f32;
        let mass_share = body.mass() / wheel_count;
        let driven = self.wheels.iter().filter(|w| w.driven).count().max(1) as f32;
        let mut forces = Vec::with_capacity(self.wheels.len());

        for wheel in &mut self.wheels {
            let origin = body.to_world(wheel.mount);
            let reach = self.rest_length + wheel.radius;
            let steer = if wheel.steers { -self.steer_angle } else { 0.0 };
            let forward = body.rot.rotate(Quat::from_axis_angle(Vec3::Y, steer).rotate(-Vec3::Z));
            let Some(hit) = phys.raycast(origin, -up, reach, RayFilter::ignoring(self.body)) else {
                wheel.compression = 0.0;
                wheel.load = 0.0;
                wheel.contact = None;
                if wheel.driven {
                    wheel.spin += input.throttle * 20.0 * dt;
                }
                continue;
            };
            wheel.compression = (reach - hit.t).min(self.rest_length);
            wheel.contact = Some((hit.point, hit.normal));

            // Relative velocity at the contact (the ground may be a moving body).
            let ground_vel = match hit.target {
                HitTarget::Particle(id) => phys.vel[id.0 as usize],
                HitTarget::Rigid(id) => phys.rigid(id).point_velocity(hit.point),
                _ => Vec3::ZERO,
            };
            let v = body.point_velocity(hit.point) - ground_vel;

            // Suspension: spring on compression, damper on compression speed. Bottoming out adds
            // a stiff bump stop instead of letting the chassis hit the ground.
            let bump = (reach - hit.t - self.rest_length).max(0.0) * self.stiffness * 4.0;
            let load = (self.stiffness * wheel.compression + bump - self.damping * v.dot(up)).max(0.0);
            wheel.load = load;

            // Tire frame on the contact plane.
            let n = hit.normal;
            let f = (forward - n * forward.dot(n)).normalize();
            let side = f.cross(n);
            let (v_long, v_lat) = (v.dot(f), v.dot(side));
            wheel.spin += v_long / wheel.radius * dt;

            // Lateral: cancel the sideways slide within one tick. Longitudinal: engine, brakes
            // (cancel forward speed within one tick, capped) and rolling resistance.
            let mut f_lat = -v_lat * mass_share / dt;
            let mut f_long = -v_long * mass_share * 0.15;
            if wheel.driven {
                f_long += input.throttle * self.engine_force / driven;
            }
            if input.brake > 0.0 {
                let cap = input.brake * self.brake_force / wheel_count;
                f_long += (-v_long * mass_share / dt).clamp(-cap, cap);
            }
            // Friction circle: the tire can't push more than grip * load in any direction.
            let max = self.grip * load;
            let total = (f_long * f_long + f_lat * f_lat).sqrt();
            if total > max {
                (f_long, f_lat) = (f_long * max / total, f_lat * max / total);
            }
            let height = (body.pos - hit.point).dot(up) * (1.0 - self.roll_influence);
            forces.push(WheelForce {
                suspension: up * load,
                contact: hit.point,
                tire: f * f_long + side * f_lat,
                tire_at: hit.point + up * height,
                ground: hit.target,
            });
        }

        let chassis = phys.rigid_mut(self.body);
        chassis.add_force_at(body.vel * (-self.air_drag * body.vel.length()), body.pos);
        for f in &forces {
            chassis.add_force_at(f.suspension, f.contact);
            chassis.add_force_at(f.tire, f.tire_at);
        }
        // Newton's third law: whatever the wheel stands on gets the opposite push.
        for f in &forces {
            let reaction = -(f.suspension + f.tire) * dt;
            match f.ground {
                HitTarget::Particle(id) => phys.apply_impulse(id, reaction),
                HitTarget::Rigid(id) => phys.rigid_mut(id).apply_impulse_at(reaction, f.contact),
                HitTarget::Bounds | HitTarget::Static(_) => {}
            }
        }
    }

    /// Pose of wheel `i` for drawing: position at the end of the suspension, rotated by the
    /// chassis, the steering and the rolling angle. The wheel's axle is local X.
    pub fn wheel_pose(&self, phys: &PhysicsWorld, i: usize) -> (Vec3, Quat) {
        let body = phys.rigid(self.body);
        let w = &self.wheels[i];
        let travel = self.rest_length - w.compression;
        let pos = body.to_world(w.mount - Vec3::Y * travel);
        let steer = if w.steers { -self.steer_angle } else { 0.0 };
        let rot = body.rot * Quat::from_axis_angle(Vec3::Y, steer) * Quat::from_axis_angle(Vec3::X, -w.spin);
        (pos, rot)
    }

    /// Puts the chassis back on its wheels at `pos`, facing `yaw`, at rest.
    pub fn reset(&mut self, phys: &mut PhysicsWorld, pos: Vec3, yaw: f32) {
        phys.rigid_mut(self.body).teleport(pos, Quat::from_axis_angle(Vec3::Y, yaw));
        self.steer_angle = 0.0;
    }
}

struct WheelForce {
    suspension: Vec3,
    contact: Vec3,
    tire: Vec3,
    tire_at: Vec3,
    ground: HitTarget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        jobs::JobSystem,
        math::vec3,
        physics::RigidBody,
    };

    fn car() -> (PhysicsWorld, RaycastVehicle) {
        let mut w = PhysicsWorld::default();
        w.bounds_min = vec3(-200.0, 0.0, -200.0);
        w.bounds_max = vec3(200.0, f32::INFINITY, 200.0);
        let half = vec3(0.9, 0.3, 2.0);
        let id = w.add_rigid(RigidBody::new_box(vec3(0.0, 1.2, 0.0), half, 1200.0));
        (w, RaycastVehicle::four_wheels(id, half, 0.4))
    }

    #[test]
    fn car_rests_on_its_suspension_then_drives_and_turns() {
        let jobs = JobSystem::new(1);
        let (mut w, mut car) = car();
        let dt = 1.0 / 60.0;
        for _ in 0..180 {
            car.update(&mut w, VehicleInput::default(), dt);
            w.step(&jobs, dt);
        }
        let body = w.rigid(car.body).clone();
        assert!(body.vel.length() < 0.05, "settled: {:?}", body.vel);
        assert!(car.wheels.iter().all(|wh| wh.load > 0.0), "all wheels on the ground");
        let expected = 1200.0 * 9.81 / 4.0 / car.stiffness;
        for wh in &car.wheels {
            assert!((wh.compression - expected).abs() < 0.03, "compression {} vs {expected}", wh.compression);
        }
        // Chassis bottom well above the floor: the car stands on its wheels, not its belly.
        assert!(body.pos.y - body.half.y > 0.2);

        for _ in 0..180 {
            car.update(&mut w, VehicleInput { throttle: 1.0, ..Default::default() }, dt);
            w.step(&jobs, dt);
        }
        let speed = car.forward_speed(&w);
        assert!(speed > 8.0, "accelerates forward (-Z): {speed}");
        assert!(w.rigid(car.body).pos.z < -10.0);

        let id = car.body;
        let heading = |w: &PhysicsWorld| {
            let f = w.rigid(id).rot.rotate(-Vec3::Z);
            f.x.atan2(-f.z)
        };
        let before = heading(&w);
        for _ in 0..90 {
            car.update(&mut w, VehicleInput { throttle: 0.5, steer: 1.0, ..Default::default() }, dt);
            w.step(&jobs, dt);
        }
        let up = w.rigid(car.body).rot.rotate(Vec3::Y);
        assert!(heading(&w) - before > 0.5, "steering right turns right");
        assert!(up.y > 0.9, "stays upright in the turn: {up:?}");

        for _ in 0..240 {
            car.update(&mut w, VehicleInput { brake: 1.0, ..Default::default() }, dt);
            w.step(&jobs, dt);
        }
        assert!(w.rigid(car.body).vel.length() < 0.2, "brakes to a stop");
    }
}
