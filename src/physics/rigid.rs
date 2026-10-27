//! Rigid boxes with position and orientation, solved with extended position-based dynamics
//! (Müller et al. 2020, "Detailed Rigid Body Simulation with Extended Position Based Dynamics"):
//! predict the pose, fix contacts by moving the bodies, then derive the velocities from the motion.
//!
//! Collision uses sample points. The 8 corners and 12 edge midpoints of each box are tested
//! against the floor and walls, against static boxes, and against other rigid bodies' boxes.
//! Particles are tested exactly (sphere vs box) by the world. Contacts are solved one at a time
//! (Gauss-Seidel), serially. There are tens of rigid bodies, not thousands, and a fixed order keeps
//! results deterministic.

use crate::{
    math::{Quat, Vec3, vec3},
    physics::shapes::{Obb, StaticBox},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RigidId(pub u32);

/// Component linking an entity to a rigid body; `physics.sync` copies its pose into `Transform`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rigid(pub RigidId);

/// One rigid box. Few of these exist and the solver touches every field of a body at once, so
/// they are stored as an array of structs.
#[derive(Clone, Debug)]
pub struct RigidBody {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub ang_vel: Vec3,
    pub half: Vec3,
    pub inv_mass: f32,
    /// Inverse of the diagonal inertia tensor, in body space.
    pub inv_inertia: Vec3,
    pub friction: f32,
    force: Vec3,
    torque: Vec3,
    prev_pos: Vec3,
    prev_rot: Quat,
}

impl RigidBody {
    /// Solid box with half extents `half`; `mass <= 0` makes it immovable.
    pub fn new_box(pos: Vec3, half: Vec3, mass: f32) -> RigidBody {
        let (inv_mass, inv_inertia) = if mass > 0.0 {
            let s = half * 2.0;
            let i = vec3(s.y * s.y + s.z * s.z, s.x * s.x + s.z * s.z, s.x * s.x + s.y * s.y) * (mass / 12.0);
            (1.0 / mass, vec3(1.0 / i.x, 1.0 / i.y, 1.0 / i.z))
        } else {
            (0.0, Vec3::ZERO)
        };
        RigidBody {
            pos,
            rot: Quat::IDENTITY,
            vel: Vec3::ZERO,
            ang_vel: Vec3::ZERO,
            half,
            inv_mass,
            inv_inertia,
            friction: 0.6,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            prev_pos: pos,
            prev_rot: Quat::IDENTITY,
        }
    }

    pub fn with_rot(mut self, rot: Quat) -> RigidBody {
        self.rot = rot;
        self.prev_rot = rot;
        self
    }

    /// Moves the body to a new pose, at rest.
    pub fn teleport(&mut self, pos: Vec3, rot: Quat) {
        (self.pos, self.rot, self.prev_pos, self.prev_rot) = (pos, rot, pos, rot);
        (self.vel, self.ang_vel) = (Vec3::ZERO, Vec3::ZERO);
    }

    pub fn is_dynamic(&self) -> bool {
        self.inv_mass > 0.0
    }

    pub fn mass(&self) -> f32 {
        if self.is_dynamic() { 1.0 / self.inv_mass } else { f32::INFINITY }
    }

    pub fn to_world(&self, local: Vec3) -> Vec3 {
        self.pos + self.rot.rotate(local)
    }

    /// Velocity of the body-fixed point currently at `p`.
    pub fn point_velocity(&self, p: Vec3) -> Vec3 {
        self.vel + self.ang_vel.cross(p - self.pos)
    }

    /// Force acting at world point `p` during the next step (spread over all its substeps).
    pub fn add_force_at(&mut self, f: Vec3, p: Vec3) {
        self.force += f;
        self.torque += (p - self.pos).cross(f);
    }

    /// Instant change of momentum at world point `p`.
    pub fn apply_impulse_at(&mut self, j: Vec3, p: Vec3) {
        if self.is_dynamic() {
            self.vel += j * self.inv_mass;
            self.ang_vel += self.inv_inertia_world((p - self.pos).cross(j));
        }
    }

    pub(crate) fn obb(&self) -> Obb {
        Obb { center: self.pos, rot: self.rot, half: self.half }
    }

    fn inv_inertia_world(&self, v: Vec3) -> Vec3 {
        self.rot.rotate(self.inv_inertia * self.rot.unrotate(v))
    }

    /// Generalized inverse mass of a positional correction along unit `n` at world point `p`.
    pub(crate) fn inv_mass_at(&self, p: Vec3, n: Vec3) -> f32 {
        if !self.is_dynamic() {
            return 0.0;
        }
        let rn = (p - self.pos).cross(n);
        self.inv_mass + rn.dot(self.inv_inertia_world(rn))
    }

    /// Moves the body as a positional impulse `j` (displacement times mass) at `p` would.
    pub(crate) fn apply_correction(&mut self, j: Vec3, p: Vec3) {
        if self.is_dynamic() {
            let r = p - self.pos;
            self.pos += j * self.inv_mass;
            self.rot = self.rot.integrate(self.inv_inertia_world(r.cross(j)));
        }
    }

    /// Body-space coordinates of world point `p`.
    fn to_local(&self, p: Vec3) -> Vec3 {
        self.rot.unrotate(p - self.pos)
    }

    pub(crate) fn integrate(&mut self, gravity: Vec3, h: f32, keep: f32, keep_ang: f32) {
        self.prev_pos = self.pos;
        self.prev_rot = self.rot;
        if !self.is_dynamic() {
            return;
        }
        self.vel = (self.vel + (gravity + self.force * self.inv_mass) * h) * keep;
        // Gyroscopic term (w x Iw), so spinning boxes precess instead of gaining energy.
        let local_w = self.rot.unrotate(self.ang_vel);
        let iw = self.rot.rotate(local_w / self.inv_inertia);
        let gyro = self.ang_vel.cross(iw);
        self.ang_vel = (self.ang_vel + self.inv_inertia_world(self.torque - gyro) * h) * keep_ang;
        self.pos += self.vel * h;
        self.rot = self.rot.integrate(self.ang_vel * h);
    }

    pub(crate) fn update_velocity(&mut self, h: f32) {
        if !self.is_dynamic() {
            return;
        }
        self.vel = (self.pos - self.prev_pos) / h;
        let dq = self.rot * self.prev_rot.conjugate();
        let w = vec3(dq.x, dq.y, dq.z) * (2.0 / h);
        self.ang_vel = if dq.w >= 0.0 { w } else { -w };
    }

    pub(crate) fn clear_forces(&mut self) {
        self.force = Vec3::ZERO;
        self.torque = Vec3::ZERO;
    }
}

/// The 26 sample points of a box: 8 corners, 12 edge midpoints and 6 face centers.
fn sample_points(h: Vec3) -> [Vec3; 26] {
    let mut pts = [Vec3::ZERO; 26];
    let mut k = 0;
    for x in [-1.0f32, 0.0, 1.0] {
        for y in [-1.0f32, 0.0, 1.0] {
            for z in [-1.0f32, 0.0, 1.0] {
                if (x, y, z) != (0.0, 0.0, 0.0) {
                    pts[k] = vec3(x * h.x, y * h.y, z * h.z);
                    k += 1;
                }
            }
        }
    }
    pts
}

/// Which face of box `b` the points of a box centered at `a` leave through: the face axis most
/// aligned with the direction between the centers, in units of `b`'s half extents. Choosing it
/// once per pair, instead of the nearest face per point, is what keeps aligned stacks standing:
/// there the corners of the top box sit exactly on the side faces of the bottom one, and "nearest
/// face" would push them sideways.
fn exit_face(b: &Obb, a: Vec3) -> (usize, f32) {
    let d = (b.rot.unrotate(a - b.center) / b.half).to_array();
    let axis = if d[0].abs() >= d[1].abs() && d[0].abs() >= d[2].abs() {
        0
    } else if d[1].abs() >= d[2].abs() {
        1
    } else {
        2
    };
    (axis, if d[axis] < 0.0 { -1.0 } else { 1.0 })
}

/// Point `p` inside box `b`: normal and depth to leave through face `(axis, sign)`.
fn face_contact(b: &Obb, (axis, sign): (usize, f32), p: Vec3) -> Option<(Vec3, f32)> {
    let q = b.rot.unrotate(p - b.center).to_array();
    let h = b.half.to_array();
    if (0..3).any(|k| q[k].abs() >= h[k]) {
        return None;
    }
    let mut n = [0.0; 3];
    n[axis] = sign;
    Some((b.rot.rotate(Vec3::from_array(n)), h[axis] - sign * q[axis]))
}

/// What rigid bodies collide with besides each other.
pub(crate) struct Environment<'a> {
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
    pub statics: &'a [StaticBox],
    pub static_friction: f32,
}

/// The other side of a contact.
enum Other<'a> {
    Fixed(f32),
    Body(&'a mut RigidBody),
}

/// A contact solved this substep, kept for the velocity pass.
pub(crate) struct Contact {
    a: usize,
    b: Option<usize>,
    local_a: Vec3,
    local_b: Vec3,
    n: Vec3,
    lambda_n: f32,
    mu: f32,
}

/// Pushes the body-space point `local` of `a` out along `n` by `depth`, then removes the
/// contact point's sliding this substep, up to Coulomb's `mu * normal` (static + kinetic friction
/// in one rule). Returns the normal correction, friction coefficient and `b`-space point.
fn solve_contact(a: &mut RigidBody, mut b: Other, local: Vec3, n: Vec3, depth: f32) -> Option<(f32, f32, Vec3)> {
    let p = a.to_world(local);
    let (wb, mu, local_b) = match &b {
        Other::Fixed(mu) => (0.0, *mu, Vec3::ZERO),
        Other::Body(b) => (b.inv_mass_at(p, n), 0.5 * (a.friction + b.friction), b.to_local(p)),
    };
    let w = a.inv_mass_at(p, n) + wb;
    if w <= 0.0 {
        return None;
    }
    let lambda_n = depth / w;
    a.apply_correction(n * lambda_n, p);
    if let Other::Body(b) = &mut b {
        b.apply_correction(n * -lambda_n, p);
    }
    Some((lambda_n, mu, local_b))
}

/// Friction, at velocity level (Müller et al. 2020, eq. 30): after velocities are derived from
/// positions, each contact removes up to `mu * normal impulse` of its sliding velocity, which
/// covers both sticking (all of it) and sliding (capped).
///
/// The paper also has position-level static friction. With sample points it made resting boxes
/// creep: every normal correction tilts the box a little, the friction correction then cancels
/// that tilt at one point by translating the whole box, and the sequential order never averages
/// out. Measured on a two-box stack over 10 s: 7 cm of drift with it, 0.2 mm without.
pub(crate) fn solve_velocities(bodies: &mut [RigidBody], contacts: &[Contact], h: f32) {
    for c in contacts {
        let p = bodies[c.a].to_world(c.local_a);
        let mut v = bodies[c.a].point_velocity(p);
        if let Some(j) = c.b {
            v -= bodies[j].point_velocity(bodies[j].to_world(c.local_b));
        }
        let vt = v - c.n * v.dot(c.n);
        let len = vt.length();
        if len < 1e-6 {
            continue;
        }
        let t = vt / len;
        let w = bodies[c.a].inv_mass_at(p, t) + c.b.map_or(0.0, |j| bodies[j].inv_mass_at(p, t));
        if w <= 0.0 {
            continue;
        }
        // lambda_n / h^2 is the normal force; times h, the normal impulse per unit inverse mass.
        let dv = len.min(c.mu * c.lambda_n / h * w);
        let impulse = t * (-dv / w);
        bodies[c.a].apply_impulse_at(impulse, p);
        if let Some(j) = c.b {
            bodies[j].apply_impulse_at(-impulse, p);
        }
    }
}

fn pair_mut<T>(v: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
    debug_assert!(i != j);
    if i < j {
        let (a, b) = v.split_at_mut(j);
        (&mut a[i], &mut b[0])
    } else {
        let (a, b) = v.split_at_mut(i);
        (&mut b[0], &mut a[j])
    }
}

/// One Gauss-Seidel sweep over every rigid contact. Odd sweeps visit bodies and points in reverse
/// order (symmetric Gauss-Seidel): a fixed order lets whoever is solved first win every time, which
/// shows up as resting boxes creeping sideways.
pub(crate) fn solve_contacts(
    bodies: &mut [RigidBody],
    env: &Environment,
    candidates: &mut Vec<Candidate>,
    contacts: &mut Vec<Contact>,
    sweep: u32,
) {
    const MARGIN: f32 = 0.1;
    let reverse = sweep % 2 == 1;
    for step in 0..bodies.len() {
        let i = if reverse { bodies.len() - 1 - step } else { step };
        if !bodies[i].is_dynamic() {
            continue;
        }
        // Broadphase by bounding spheres, once per body per sweep.
        candidates.clear();
        let (pi, ri) = (bodies[i].pos, bodies[i].obb().bound_radius() + MARGIN);
        for (k, s) in env.statics.iter().enumerate() {
            let reach = ri + s.half.length();
            if (s.center - pi).length_sq() < reach * reach {
                candidates.push(Candidate::Static(k, exit_face(&s.obb(), pi)));
            }
        }
        for (j, b) in bodies.iter().enumerate() {
            let reach = ri + b.obb().bound_radius();
            if j != i && (b.pos - pi).length_sq() < reach * reach {
                candidates.push(Candidate::Body(j, exit_face(&b.obb(), pi)));
            }
        }
        let near_bounds = pi.y - ri < env.bounds_min.y
            || pi.x - ri < env.bounds_min.x
            || pi.x + ri > env.bounds_max.x
            || pi.z - ri < env.bounds_min.z
            || pi.z + ri > env.bounds_max.z;
        if candidates.is_empty() && !near_bounds {
            continue;
        }

        let mut points = sample_points(bodies[i].half);
        if reverse {
            points.reverse();
        }
        for local in points {
            if near_bounds {
                for (n, lo) in [(Vec3::Y, env.bounds_min.y), (Vec3::X, env.bounds_min.x), (Vec3::Z, env.bounds_min.z)] {
                    let depth = lo - bodies[i].to_world(local).dot(n);
                    if depth > 0.0 {
                        record(contacts, i, None, local, n, solve_contact(&mut bodies[i], Other::Fixed(env.static_friction), local, n, depth));
                    }
                }
                for (n, hi) in [(-Vec3::X, env.bounds_max.x), (-Vec3::Z, env.bounds_max.z)] {
                    let depth = bodies[i].to_world(local).dot(-n) - hi;
                    if depth > 0.0 {
                        record(contacts, i, None, local, n, solve_contact(&mut bodies[i], Other::Fixed(env.static_friction), local, n, depth));
                    }
                }
            }
            for c in candidates.iter() {
                let p = bodies[i].to_world(local);
                match *c {
                    Candidate::Static(k, face) => {
                        if let Some((n, depth)) = face_contact(&env.statics[k].obb(), face, p) {
                            record(contacts, i, None, local, n, solve_contact(&mut bodies[i], Other::Fixed(env.static_friction), local, n, depth));
                        }
                    }
                    Candidate::Body(j, face) => {
                        if let Some((n, depth)) = face_contact(&bodies[j].obb(), face, p) {
                            let (a, b) = pair_mut(bodies, i, j);
                            record(contacts, i, Some(j), local, n, solve_contact(a, Other::Body(b), local, n, depth));
                        }
                    }
                }
            }
        }
    }
}

fn record(contacts: &mut Vec<Contact>, a: usize, b: Option<usize>, local_a: Vec3, n: Vec3, solved: Option<(f32, f32, Vec3)>) {
    if let Some((lambda_n, mu, local_b)) = solved {
        contacts.push(Contact { a, b, local_a, local_b, n, lambda_n, mu });
    }
}

/// A box near the one being solved, with the face its points leave through.
#[derive(Clone, Copy)]
pub(crate) enum Candidate {
    Static(usize, (usize, f32)),
    Body(usize, (usize, f32)),
}
