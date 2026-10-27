//! Position-based dynamics (PBD) inside an axis-aligned box: thousands of spheres ("particles"),
//! tens of rigid boxes ([`rigid`]), distance constraints between particles ([`constraints`]),
//! static boxes ([`shapes`]), and the queries game systems use ([`query`]: raycasts, explosions).
//! Gameplay mechanisms built only on that public API live next to it: [`vehicle`], [`water`].
//!
//! Storage is structure-of-arrays owned by `PhysicsWorld` (the way Jolt/PhysX own their bodies);
//! entities point at a body through the `Body`/`Rigid` components and `sync` copies results back.
//!
//! One step = `substeps` substeps (small steps beat many iterations, Macklin et al. 2019). The
//! particle passes are data-parallel loops over bodies:
//!
//! 1. integrate: `prev = pos; vel += g*h; pos += vel*h`
//! 2. broadphase: spatial hash grid built with a counting sort, then one neighbour list per body
//!    (everything within contact distance + a margin). The 27-cell hash search runs once per
//!    substep; solver iterations only walk the compact lists.
//! 3. contacts (Jacobi): each body *gathers* corrections from its neighbours and writes only its
//!    own slot — no scatter, no atomics, no locks — then all corrections are applied at once.
//!    Neighbours are tested 8 at a time with SIMD (`F32x8` gathers of position/radius/mass)
//!    Then, serially and in a fixed order: distance constraints, rigid contacts (box sample points
//!    vs floor/static boxes/other boxes) and rigid-vs-particle contacts.
//! 4. statics: project particles out of the walls, floor and static boxes
//! 5. velocities from positions: `vel = (pos - prev) / h`, plus friction on static surfaces
//!
//! Because no pass depends on how work is split, results are bit-identical for any thread count.

pub mod constraints;
pub mod query;
pub mod rigid;
pub mod shapes;
pub mod vehicle;
pub mod water;

pub use constraints::DistanceConstraint;
pub use query::{HitTarget, RayFilter, RayHit};
pub use rigid::{Rigid, RigidBody, RigidId};
pub use shapes::StaticBox;

use crate::{
    jobs::Jobs,
    math::{Vec3, vec3},
    profile,
    physics::shapes::Obb,
    scene::Transform,
    schedule::{System, SystemCtx, system},
    simd::{F32x8, I32x8},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct BodyId(pub u32);

/// Component linking an entity to its physics body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Body(pub BodyId);

const BATCH: usize = 512;
/// Neighbour list capacity per body. Touching spheres of similar size have at most ~12-20
/// neighbours, so this only truncates in degenerate pile-ups.
const MAX_NEIGHBORS: usize = 32;

pub struct PhysicsWorld {
    pub gravity: Vec3,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
    pub substeps: u32,
    pub iterations: u32,
    /// Jacobi over-relaxation (1 = plain average of the contact corrections).
    pub relaxation: f32,
    /// Fraction of tangential velocity removed per substep while touching the floor.
    pub friction: f32,
    /// Linear damping per second.
    pub damping: f32,
    /// Cap on the separating speed a contact correction may add beyond cancelling the approach.
    /// Without it, overlap that wasn't caused by this substep's motion (e.g. a body squeezed into
    /// a corner) turns into launch velocity.
    pub max_depenetration_speed: f32,
    /// Extra distance, as a fraction of the largest radius, for a pair to enter the neighbour
    /// list. Covers how far bodies move between the list build and the last solver iteration.
    pub neighbor_margin: f32,

    pub pos: Vec<Vec3>,
    pub vel: Vec<Vec3>,
    pub radius: Vec<f32>,
    pub inv_mass: Vec<f32>,
    /// Particles sharing a non-zero group never collide with each other (a rope's links, a net's
    /// knots); they still collide with everything else.
    pub group: Vec<u32>,

    pub statics: Vec<StaticBox>,
    pub rigids: Vec<RigidBody>,
    pub constraints: Vec<DistanceConstraint>,
    /// Friction coefficient of the floor, walls and static boxes against rigid bodies.
    pub static_friction: f32,
    /// Angular damping of rigid bodies per second.
    pub angular_damping: f32,

    prev: Vec<Vec3>,
    delta: Vec<Vec3>,
    /// Sum of contact corrections applied to each body during the current substep.
    push: Vec<Vec3>,
    grid: HashGrid,
    neighbors: Vec<[u32; MAX_NEIGHBORS]>,
    neighbor_count: Vec<u32>,
    /// Normal of the static surface each particle touched this substep (zero if none).
    touch: Vec<Vec3>,
    candidates: Vec<rigid::Candidate>,
    rigid_contacts: Vec<rigid::Contact>,
    max_radius: f32,
}

impl Default for PhysicsWorld {
    fn default() -> PhysicsWorld {
        PhysicsWorld {
            gravity: vec3(0.0, -9.81, 0.0),
            bounds_min: vec3(-10.0, 0.0, -10.0),
            bounds_max: vec3(10.0, f32::INFINITY, 10.0),
            substeps: 4,
            iterations: 2,
            relaxation: 1.0,
            friction: 0.01,
            damping: 0.05,
            max_depenetration_speed: 1.0,
            neighbor_margin: 0.25,
            pos: Vec::new(),
            vel: Vec::new(),
            radius: Vec::new(),
            inv_mass: Vec::new(),
            group: Vec::new(),
            statics: Vec::new(),
            rigids: Vec::new(),
            constraints: Vec::new(),
            static_friction: 0.7,
            angular_damping: 0.1,
            prev: Vec::new(),
            delta: Vec::new(),
            push: Vec::new(),
            grid: HashGrid::default(),
            neighbors: Vec::new(),
            neighbor_count: Vec::new(),
            touch: Vec::new(),
            candidates: Vec::new(),
            rigid_contacts: Vec::new(),
            max_radius: 0.0,
        }
    }
}

impl PhysicsWorld {
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// Sphere particle; `mass <= 0` makes it static (a pinned rope end, say).
    pub fn add_body(&mut self, pos: Vec3, radius: f32, mass: f32) -> BodyId {
        self.add_particle(pos, radius, mass, 0)
    }

    /// Like `add_body`, in a collision group (see `group`).
    pub fn add_particle(&mut self, pos: Vec3, radius: f32, mass: f32, group: u32) -> BodyId {
        let id = BodyId(self.pos.len() as u32);
        self.pos.push(pos);
        self.vel.push(Vec3::ZERO);
        self.radius.push(radius);
        self.inv_mass.push(if mass > 0.0 { 1.0 / mass } else { 0.0 });
        self.group.push(group);
        self.max_radius = self.max_radius.max(radius);
        id
    }

    /// Instant change of a particle's momentum.
    pub fn apply_impulse(&mut self, id: BodyId, j: Vec3) {
        let i = id.0 as usize;
        self.vel[i] += j * self.inv_mass[i];
    }

    pub fn add_static(&mut self, s: StaticBox) {
        self.statics.push(s);
    }

    pub fn add_rigid(&mut self, body: RigidBody) -> RigidId {
        self.rigids.push(body);
        RigidId(self.rigids.len() as u32 - 1)
    }

    pub fn rigid(&self, id: RigidId) -> &RigidBody {
        &self.rigids[id.0 as usize]
    }

    pub fn rigid_mut(&mut self, id: RigidId) -> &mut RigidBody {
        &mut self.rigids[id.0 as usize]
    }

    pub fn add_constraint(&mut self, c: DistanceConstraint) {
        self.constraints.push(c);
    }

    /// Heap bytes reserved per part of the solver.
    pub fn memory(&self) -> Vec<(&'static str, usize)> {
        fn b<T>(v: &Vec<T>) -> usize {
            v.capacity() * size_of::<T>()
        }
        let g = &self.grid;
        vec![
            (
                "physics.particles",
                b(&self.pos) + b(&self.vel) + b(&self.radius) + b(&self.inv_mass) + b(&self.group)
                    + b(&self.prev) + b(&self.delta) + b(&self.push) + b(&self.touch),
            ),
            ("physics.neighbors", b(&self.neighbors) + b(&self.neighbor_count)),
            ("physics.grid", b(&g.cells) + b(&g.hashes) + b(&g.bucket_start) + b(&g.cursor) + b(&g.entries)),
            ("physics.rigid", b(&self.rigids) + b(&self.rigid_contacts) + b(&self.candidates) + b(&self.statics)),
            ("physics.constraints", b(&self.constraints)),
        ]
    }

    pub fn kinetic_energy(&self) -> f32 {
        self.vel.iter().zip(&self.inv_mass).filter(|(_, w)| **w > 0.0).map(|(v, w)| 0.5 * v.length_sq() / w).sum()
    }

    pub fn step(&mut self, jobs: &Jobs, dt: f32) {
        let n = self.pos.len();
        if n == 0 && self.rigids.is_empty() {
            return;
        }
        self.prev.resize(n, Vec3::ZERO);
        self.delta.resize(n, Vec3::ZERO);
        self.push.resize(n, Vec3::ZERO);
        self.touch.resize(n, Vec3::ZERO);
        self.neighbors.resize(n, [0; MAX_NEIGHBORS]);
        self.neighbor_count.resize(n, 0);
        let h = dt / self.substeps as f32;
        let (keep, keep_ang) = ((1.0 - self.damping * h).max(0.0), (1.0 - self.angular_damping * h).max(0.0));
        for _ in 0..self.substeps {
            self.integrate(jobs, h);
            for body in &mut self.rigids {
                body.integrate(self.gravity, h, keep, keep_ang);
            }
            self.rigid_contacts.clear();
            if n > 0 {
                let buckets = (2 * n).next_power_of_two().max(64);
                let _p = profile::scope_bytes("physics.broadphase", n * 220 + buckets * 8);
                self.grid.build(jobs, &self.pos, &self.radius, 2.0 * self.max_radius);
                self.build_neighbor_lists(jobs);
            }
            for sweep in 0..self.iterations {
                if n > 0 {
                    self.solve_contacts(jobs);
                }
                if !self.constraints.is_empty() {
                    let _p = profile::scope_bytes("physics.constraints", self.constraints.len() * 52);
                    constraints::solve(&self.constraints, &mut self.pos, &self.inv_mass, h);
                }
                if !self.rigids.is_empty() {
                    let _p = profile::scope_bytes(
                        "physics.rigid",
                        self.rigids.len() * size_of::<RigidBody>() + self.statics.len() * size_of::<StaticBox>(),
                    );
                    let env = rigid::Environment {
                        bounds_min: self.bounds_min,
                        bounds_max: self.bounds_max,
                        statics: &self.statics,
                        static_friction: self.static_friction,
                    };
                    rigid::solve_contacts(&mut self.rigids, &env, &mut self.candidates, &mut self.rigid_contacts, sweep);
                    if n > 0 {
                        self.solve_rigid_particle_contacts();
                    }
                }
            }
            self.solve_statics(jobs);
            self.update_velocities(jobs, h);
            for body in &mut self.rigids {
                body.update_velocity(h);
            }
            rigid::solve_velocities(&mut self.rigids, &self.rigid_contacts, h);
        }
        for body in &mut self.rigids {
            body.clear_forces();
        }
    }

    fn integrate(&mut self, jobs: &Jobs, h: f32) {
        let _p = profile::scope_bytes("physics.integrate", self.pos.len() * 52);
        let g = self.gravity * h;
        let keep = (1.0 - self.damping * h).max(0.0);
        jobs.par_zip((&mut self.pos, &mut self.vel, &mut self.prev, &mut self.push, &self.inv_mass), BATCH, |_, (pos, vel, prev, push, w)| {
            for i in 0..pos.len() {
                prev[i] = pos[i];
                push[i] = Vec3::ZERO;
                if w[i] == 0.0 {
                    continue;
                }
                vel[i] = vel[i] * keep + g;
                pos[i] += vel[i] * h;
            }
        });
    }

    fn build_neighbor_lists(&mut self, jobs: &Jobs) {
        let (pos, radius, inv_mass, group, grid) = (&self.pos, &self.radius, &self.inv_mass, &self.group, &self.grid);
        let margin = self.neighbor_margin * self.max_radius;
        jobs.par_zip((&mut self.neighbors, &mut self.neighbor_count), BATCH, |start, (lists, counts)| {
            for (k, (list, count)) in lists.iter_mut().zip(counts.iter_mut()).enumerate() {
                let i = start + k;
                *count = 0;
                if inv_mass[i] == 0.0 {
                    continue; // static bodies are pushed by others, never corrected themselves
                }
                let (pi, ri, gi) = (pos[i], radius[i] + margin, group[i]);
                grid.for_each_neighbor(i, |e| {
                    let reach = ri + e.radius;
                    let same_group = gi != 0 && group[e.index as usize] == gi;
                    if e.index as usize != i && !same_group && (pi - e.pos).length_sq() < reach * reach && (*count as usize) < MAX_NEIGHBORS {
                        list[*count as usize] = e.index;
                        *count += 1;
                    }
                });
            }
        });
    }

    fn solve_contacts(&mut self, jobs: &Jobs) {
        // Neighbour lists, gathered position/radius/mass, delta, then pos/push/delta again.
        let _p = profile::scope_bytes("physics.contacts", self.pos.len() * 200);
        let bodies = Bodies { pos: vec3_words(&self.pos), radius: &self.radius, inv_mass: &self.inv_mass };
        let relaxation = self.relaxation;
        jobs.par_zip((&mut self.delta, &self.neighbors, &self.neighbor_count), BATCH, |start, (delta, lists, counts)| {
            for k in 0..delta.len() {
                let i = start + k;
                delta[k] = Vec3::ZERO;
                let list = &lists[k][..counts[k] as usize];
                if list.is_empty() {
                    continue;
                }
                let mut sum = ContactSum::new(self.pos[i], bodies.radius[i], bodies.inv_mass[i], i);
                for batch in list.chunks(8) {
                    let mut js = [batch[0]; 8]; // unused lanes repeat a valid index
                    js[..batch.len()].copy_from_slice(batch);
                    sum.add8(&bodies, js, batch.len());
                }
                if sum.count > 0 {
                    delta[k] = sum.total() * (relaxation / sum.count as f32);
                }
            }
        });
        jobs.par_zip((&mut self.pos, &mut self.push, &self.delta), BATCH, |_, (pos, push, delta)| {
            for i in 0..pos.len() {
                pos[i] += delta[i];
                push[i] += delta[i];
            }
        });
    }

    /// Particles against rigid boxes, exact sphere-vs-box. Corrections don't count as contact
    /// `push`: a box hitting a ball hands it the box's speed instead of the gentle depenetration.
    fn solve_rigid_particle_contacts(&mut self) {
        let (grid, pos, radius, inv_mass) = (&self.grid, &mut self.pos, &self.radius, &self.inv_mass);
        let reach_extra = self.max_radius + 1.0 / grid.inv_cell;
        for body in &mut self.rigids {
            let reach = Vec3::splat(body.obb().bound_radius() + reach_extra);
            grid.for_each_in_aabb(body.pos - reach, body.pos + reach, |i| {
                let Some((n, depth)) = body.obb().sphere_contact(pos[i], radius[i]) else {
                    return;
                };
                let p = pos[i] - n * radius[i];
                let w = inv_mass[i] + body.inv_mass_at(p, n);
                if w > 0.0 {
                    let lambda = depth / w;
                    pos[i] += n * (lambda * inv_mass[i]);
                    body.apply_correction(n * -lambda, p);
                }
            });
        }
    }

    /// Walls, floor and static boxes. Records the surface normal for friction.
    fn solve_statics(&mut self, jobs: &Jobs) {
        let _p = profile::scope_bytes("physics.statics", self.pos.len() * 32);
        let (lo, hi) = (self.bounds_min, self.bounds_max);
        let statics: Vec<(Obb, f32)> = self.statics.iter().map(|s| (s.obb(), s.half.length())).collect();
        jobs.par_zip((&mut self.pos, &mut self.touch, &self.radius, &self.inv_mass), BATCH, |_, (pos, touch, radius, w)| {
            for i in 0..pos.len() {
                touch[i] = Vec3::ZERO;
                if w[i] == 0.0 {
                    continue;
                }
                let r = radius[i];
                let mut p = pos[i];
                let clamped = p.max(lo + Vec3::splat(r)).min(hi - Vec3::splat(r));
                if clamped != p {
                    touch[i] = (clamped - p).normalize();
                    p = clamped;
                }
                for (obb, bound) in &statics {
                    let reach = bound + r;
                    if (obb.center - p).length_sq() < reach * reach
                        && let Some((n, depth)) = obb.sphere_contact(p, r)
                    {
                        p += n * depth;
                        touch[i] = n;
                    }
                }
                pos[i] = p;
            }
        });
    }

    fn update_velocities(&mut self, jobs: &Jobs, h: f32) {
        let _p = profile::scope_bytes("physics.velocities", self.pos.len() * 64);
        let inv_h = 1.0 / h;
        let keep = 1.0 - self.friction;
        let max_sep = self.max_depenetration_speed;
        jobs.par_zip(
            (&mut self.vel, &self.pos, &self.prev, &self.push, &self.touch, &self.inv_mass),
            BATCH,
            |_, (vel, pos, prev, push, touch, w)| {
                for i in 0..vel.len() {
                    if w[i] == 0.0 {
                        continue;
                    }
                    let mut v = (pos[i] - prev[i]) * inv_h;
                    // vel[i] still holds the integrated (pre-contact) velocity. Contacts may cancel
                    // the approach along their push direction, plus at most `max_sep` on top.
                    let push_len = push[i].length();
                    if push_len > 0.0 {
                        let dir = push[i] / push_len;
                        let allowed = (-vel[i].dot(dir)).max(0.0) + max_sep;
                        let excess = push_len * inv_h - allowed;
                        if excess > 0.0 {
                            v -= dir * excess;
                        }
                    }
                    let n = touch[i];
                    if n != Vec3::ZERO {
                        let vn = n * v.dot(n);
                        v = vn + (v - vn) * keep;
                    }
                    vel[i] = v;
                }
            },
        );
    }
}

/// Body data copied into grid order, so a bucket's bodies are contiguous in memory and the
/// neighbour search streams through them instead of jumping around `pos[j]`.
#[derive(Clone, Copy, Default)]
struct GridEntry {
    pos: Vec3,
    radius: f32,
    index: u32,
    cell: [i32; 3],
}

/// Body arrays as flat words for SIMD gathers.
struct Bodies<'a> {
    pos: &'a [f32],
    radius: &'a [f32],
    inv_mass: &'a [f32],
}

fn vec3_words(v: &[Vec3]) -> &[f32] {
    // SAFETY: Vec3 is repr(C) with three f32 fields and no padding.
    const _: () = assert!(size_of::<Vec3>() == 12);
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), v.len() * 3) }
}

/// Accumulates the contact corrections of one body against batches of 8 neighbours.
struct ContactSum {
    pi: (F32x8, F32x8, F32x8),
    ri: F32x8,
    wi: F32x8,
    i: I32x8,
    sum: (F32x8, F32x8, F32x8),
    count: u32,
}

impl ContactSum {
    fn new(p: Vec3, r: f32, w: f32, i: usize) -> ContactSum {
        let zero = F32x8::splat(0.0);
        ContactSum {
            pi: (F32x8::splat(p.x), F32x8::splat(p.y), F32x8::splat(p.z)),
            ri: F32x8::splat(r),
            wi: F32x8::splat(w),
            i: I32x8::splat(i as i32),
            sum: (zero, zero, zero),
            count: 0,
        }
    }

    /// Tests the first `n` bodies of `js`; the remaining lanes are ignored.
    #[inline(always)]
    fn add8(&mut self, b: &Bodies, js: [u32; 8], n: usize) {
        let zero = F32x8::splat(0.0);
        let j = I32x8::from_array(js.map(|j| j as i32));
        let p3 = j * I32x8::splat(3);
        let get = |k: i32| F32x8::gather(b.pos, p3 + I32x8::splat(k));
        let mut d = (self.pi.0 - get(0), self.pi.1 - get(1), self.pi.2 - get(2));
        let mut d2 = d.0.mul_add(d.0, d.1.mul_add(d.1, d.2 * d.2));
        let rr = self.ri + F32x8::gather(b.radius, j);
        let lanes = I32x8::splat(n as i32).gt(I32x8::ramp(0));
        let contact = lanes & d2.lt(rr * rr);
        if !contact.any() {
            return;
        }
        let coincident = contact & d2.lt(F32x8::splat(1e-12));
        if coincident.any() {
            // Same center: separate along x, direction fixed by the indices.
            let push = j.gt(self.i).select(F32x8::splat(1e-4), F32x8::splat(-1e-4));
            d = (coincident.select(push, d.0), coincident.select(zero, d.1), coincident.select(zero, d.2));
            d2 = coincident.select(F32x8::splat(1e-8), d2);
        }
        let dist = d2.sqrt();
        let share = self.wi / (self.wi + F32x8::gather(b.inv_mass, j));
        let f = contact.select((rr - dist) / dist * share, zero);
        self.sum = (d.0.mul_add(f, self.sum.0), d.1.mul_add(f, self.sum.1), d.2.mul_add(f, self.sum.2));
        self.count += contact.count();
    }

    fn total(&self) -> Vec3 {
        vec3(self.sum.0.sum(), self.sum.1.sum(), self.sum.2.sum())
    }
}

/// Dense spatial hash: cell -> hash bucket -> contiguous range of entries.
#[derive(Default)]
struct HashGrid {
    inv_cell: f32,
    mask: usize,
    /// Per body: its cell and that cell's bucket.
    cells: Vec<[i32; 3]>,
    hashes: Vec<u32>,
    bucket_start: Vec<u32>,
    cursor: Vec<u32>,
    entries: Vec<GridEntry>,
}

fn cell_of(p: Vec3, inv_cell: f32) -> [i32; 3] {
    [(p.x * inv_cell).floor() as i32, (p.y * inv_cell).floor() as i32, (p.z * inv_cell).floor() as i32]
}

fn hash_cell(c: [i32; 3], mask: usize) -> usize {
    let h = c[0].wrapping_mul(92_837_111) ^ c[1].wrapping_mul(689_287_499) ^ c[2].wrapping_mul(283_923_481);
    h as u32 as usize & mask
}

impl HashGrid {
    /// `cell` must be >= the largest body diameter so neighbours are always in adjacent cells.
    fn build(&mut self, jobs: &Jobs, pos: &[Vec3], radius: &[f32], cell: f32) {
        let n = pos.len();
        let size = (2 * n).next_power_of_two().max(64);
        self.mask = size - 1;
        self.inv_cell = 1.0 / cell;
        self.cells.resize(n, [0; 3]);
        self.hashes.resize(n, 0);
        let (inv_cell, mask) = (self.inv_cell, self.mask);
        jobs.par_zip((&mut self.cells, &mut self.hashes, pos), 2048, |_, (cells, hashes, pos)| {
            for i in 0..cells.len() {
                cells[i] = cell_of(pos[i], inv_cell);
                hashes[i] = hash_cell(cells[i], mask) as u32;
            }
        });
        // Counting sort. Sequential on purpose: O(n), cache friendly, and keeps bodies in index
        // order inside each bucket, which makes the solver deterministic.
        self.bucket_start.clear();
        self.bucket_start.resize(size + 1, 0);
        for &h in &self.hashes {
            self.bucket_start[h as usize] += 1;
        }
        let mut sum = 0;
        for c in self.bucket_start.iter_mut() {
            let count = *c;
            *c = sum;
            sum += count;
        }
        self.cursor.clone_from(&self.bucket_start);
        self.entries.resize(n, GridEntry::default());
        for (i, &h) in self.hashes.iter().enumerate() {
            let slot = &mut self.cursor[h as usize];
            self.entries[*slot as usize] =
                GridEntry { pos: pos[i], radius: radius[i], index: i as u32, cell: self.cells[i] };
            *slot += 1;
        }
    }

    /// Calls `f` with the index of every body whose grid cell overlaps the box `lo..hi`, each
    /// once, in grid order.
    fn for_each_in_aabb(&self, lo: Vec3, hi: Vec3, mut f: impl FnMut(usize)) {
        let (a, b) = (cell_of(lo, self.inv_cell), cell_of(hi, self.inv_cell));
        let cells = (0..3).map(|k| (b[k] - a[k] + 1) as usize).product::<usize>();
        if cells > self.entries.len() {
            for e in &self.entries {
                if (0..3).all(|k| e.cell[k] >= a[k] && e.cell[k] <= b[k]) {
                    f(e.index as usize);
                }
            }
            return;
        }
        for z in a[2]..=b[2] {
            for y in a[1]..=b[1] {
                for x in a[0]..=b[0] {
                    let target = [x, y, z];
                    let h = hash_cell(target, self.mask);
                    let (s, t) = (self.bucket_start[h] as usize, self.bucket_start[h + 1] as usize);
                    for e in &self.entries[s..t] {
                        if e.cell == target {
                            f(e.index as usize);
                        }
                    }
                }
            }
        }
    }

    /// Calls `f` for every body in the 27 cells around body `i`'s cell (including `i` itself),
    /// each exactly once, in grid order.
    fn for_each_neighbor(&self, i: usize, mut f: impl FnMut(&GridEntry)) {
        let c = self.cells[i];
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let target = [c[0] + dx, c[1] + dy, c[2] + dz];
                    let h = hash_cell(target, self.mask);
                    let (a, b) = (self.bucket_start[h] as usize, self.bucket_start[h + 1] as usize);
                    // A bucket can hold other cells (hash collisions, or two of the 27 cells sharing
                    // a bucket); matching the exact cell visits every body once.
                    for e in &self.entries[a..b] {
                        if e.cell == target {
                            f(e);
                        }
                    }
                }
            }
        }
    }
}

// --- ECS glue ------------------------------------------------------------------------------------

pub fn step_system(ctx: &SystemCtx) {
    ctx.res_mut::<PhysicsWorld>().step(ctx.jobs, ctx.frame.dt);
}

pub fn sync_transforms_system(ctx: &SystemCtx) {
    let physics = ctx.res::<PhysicsWorld>();
    let pos = &physics.pos;
    for table in ctx.query::<(Transform, Body)>() {
        let mut transforms = table.write::<Transform>();
        let bodies = table.read::<Body>();
        ctx.jobs.par_zip((&mut *transforms, &*bodies), 1024, |_, (t, b)| {
            for i in 0..t.len() {
                t[i].pos = pos[b[i].0.0 as usize];
            }
        });
    }
    for table in ctx.query::<(Transform, Rigid)>() {
        let mut transforms = table.write::<Transform>();
        for (t, r) in transforms.iter_mut().zip(table.read::<Rigid>().iter()) {
            let body = physics.rigid(r.0);
            (t.pos, t.rot) = (body.pos, body.rot);
        }
    }
}

/// `physics.step` then `physics.sync`. Add them to the fixed-step schedule.
pub fn systems() -> [System; 2] {
    [
        system("physics.step", step_system).writes::<PhysicsWorld>(),
        system("physics.sync", sync_transforms_system).reads::<PhysicsWorld>().reads::<Body>().reads::<Rigid>().writes::<Transform>(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        jobs::JobSystem,
        math::{Quat, Rng},
    };

    fn pile(n: usize) -> PhysicsWorld {
        let mut w = PhysicsWorld::default();
        w.bounds_min = vec3(-3.0, 0.0, -3.0);
        w.bounds_max = vec3(3.0, f32::INFINITY, 3.0);
        let mut rng = Rng::new(7);
        for _ in 0..n {
            let p = vec3(rng.range(-2.5, 2.5), rng.range(1.0, 12.0), rng.range(-2.5, 2.5));
            w.add_body(p, rng.range(0.2, 0.35), 1.0);
        }
        w
    }

    #[test]
    fn results_do_not_depend_on_thread_count() {
        let mut outputs = Vec::new();
        for workers in [0, 3, 7] {
            let jobs = JobSystem::new(workers);
            let mut w = pile(1500);
            for _ in 0..60 {
                w.step(&jobs, 1.0 / 60.0);
            }
            outputs.push(w.pos);
        }
        assert!(outputs.windows(2).all(|o| o[0] == o[1]));
    }

    #[test]
    fn pile_settles_inside_bounds_without_deep_overlap() {
        let jobs = JobSystem::new(3);
        let mut w = pile(400);
        for _ in 0..600 {
            w.step(&jobs, 1.0 / 60.0);
        }
        let mut worst = 0.0f32;
        for i in 0..w.len() {
            assert!(w.pos[i].y >= w.radius[i] - 1e-3);
            for j in i + 1..w.len() {
                let overlap = w.radius[i] + w.radius[j] - (w.pos[i] - w.pos[j]).length();
                worst = worst.max(overlap / w.radius[i].min(w.radius[j]));
            }
        }
        let ke = w.kinetic_energy() / w.len() as f32;
        assert!(worst < 0.25, "worst relative overlap {worst}");
        assert!(ke < 0.05, "still moving: {ke} J/body");
    }

    fn open_world() -> PhysicsWorld {
        let mut w = PhysicsWorld::default();
        w.bounds_min = vec3(-30.0, 0.0, -30.0);
        w.bounds_max = vec3(30.0, f32::INFINITY, 30.0);
        w
    }

    /// Pyramid of 5-4-3-2-1 crates.
    fn pyramid(w: &mut PhysicsWorld, base: Vec3, half: f32) -> Vec<RigidId> {
        let mut ids = Vec::new();
        for row in 0..5 {
            for k in 0..5 - row {
                let x = (k as f32 - (4 - row) as f32 * 0.5) * (2.0 * half + 0.02);
                let p = base + vec3(x, half + row as f32 * 2.0 * half, 0.0);
                ids.push(w.add_rigid(RigidBody::new_box(p, Vec3::splat(half), 150.0)));
            }
        }
        ids
    }

    #[test]
    fn crate_pyramid_stands_still() {
        let jobs = JobSystem::new(1);
        let mut w = open_world();
        let ids = pyramid(&mut w, Vec3::ZERO, 0.45);
        let start: Vec<Vec3> = ids.iter().map(|id| w.rigid(*id).pos).collect();
        for _ in 0..600 {
            w.step(&jobs, 1.0 / 60.0);
        }
        for (id, p0) in ids.iter().zip(&start) {
            let b = w.rigid(*id);
            let drift = (b.pos - *p0).length();
            assert!(drift < 0.02, "crate {} drifted {drift:.3} m", id.0);
            assert!(b.vel.length() < 0.05 && b.ang_vel.length() < 0.05, "crate {} still moving", id.0);
            assert!(b.rot.rotate(Vec3::Y).y > 0.999, "crate {} tilted", id.0);
        }
    }

    #[test]
    fn box_rests_on_a_gentle_ramp_and_slides_down_a_steep_one() {
        let jobs = JobSystem::new(1);
        let mut w = open_world();
        let mut boxes = Vec::new();
        for (x, angle) in [(-5.0f32, 0.3f32), (5.0, 0.9)] {
            let rot = Quat::from_axis_angle(Vec3::Z, angle);
            w.add_static(StaticBox::new(vec3(x, 3.0, 0.0), vec3(3.0, 0.2, 2.0)).with_rot(rot));
            let on_top = vec3(x, 3.0, 0.0) + rot.rotate(vec3(0.0, 0.2 + 0.5, 0.0));
            boxes.push(w.add_rigid(RigidBody::new_box(on_top, Vec3::splat(0.5), 50.0).with_rot(rot)));
        }
        let start: Vec<Vec3> = boxes.iter().map(|b| w.rigid(*b).pos).collect();
        for _ in 0..90 {
            w.step(&jobs, 1.0 / 60.0);
        }
        // friction 0.6..0.7: tan(0.3) = 0.31 holds, tan(0.9) = 1.26 slides.
        let gentle = (w.rigid(boxes[0]).pos - start[0]).length();
        let steep = (w.rigid(boxes[1]).pos - start[1]).length();
        assert!(gentle < 0.05, "box on the gentle ramp moved {gentle:.3} m");
        assert!(steep > 1.0, "box on the steep ramp only moved {steep:.3} m");
    }

    #[test]
    fn tethered_chain_holds_a_heavy_weight_without_stretching() {
        let jobs = JobSystem::new(1);
        let mut w = open_world();
        let (links, spacing) = (16u32, 0.3);
        let anchor = vec3(0.0, 10.0, 0.0);
        let first = w.pos.len() as u32;
        for k in 0..=links {
            let mass = if k == 0 { 0.0 } else if k == links { 400.0 } else { 5.0 };
            // Start horizontal so it swings.
            w.add_particle(anchor + vec3(k as f32 * spacing, 0.0, 0.0), 0.12, mass, 1);
        }
        for k in 0..links {
            w.add_constraint(DistanceConstraint::rod(first + k, first + k + 1, spacing));
            w.add_constraint(DistanceConstraint::tether(first, first + k + 1, spacing * (k + 1) as f32));
        }
        let length = links as f32 * spacing;
        let mut worst = 0.0f32;
        for _ in 0..240 {
            w.step(&jobs, 1.0 / 60.0);
            let d = (w.pos[(first + links) as usize] - anchor).length();
            worst = worst.max(d / length);
        }
        assert!(worst < 1.02, "chain stretched to {:.1}% of its length", worst * 100.0);
    }

    #[test]
    fn moving_box_shoves_balls_out_of_the_way() {
        let jobs = JobSystem::new(3);
        let mut w = open_world();
        let mut rng = Rng::new(3);
        for _ in 0..300 {
            w.add_body(vec3(rng.range(-2.0, 2.0), rng.range(0.3, 2.0), rng.range(-12.0, -8.0)), 0.25, 20.0);
        }
        let plow = w.add_rigid(RigidBody::new_box(vec3(0.0, 0.8, 0.0), vec3(1.5, 0.8, 0.5), 3000.0));
        for _ in 0..120 {
            w.rigid_mut(plow).vel = vec3(0.0, w.rigid(plow).vel.y, -8.0);
            w.step(&jobs, 1.0 / 60.0);
        }
        let b = w.rigid(plow).obb();
        let worst = (0..w.len()).filter_map(|i| b.sphere_contact(w.pos[i], w.radius[i])).map(|(_, d)| d).fold(0.0, f32::max);
        assert!(w.rigid(plow).pos.z < -10.0, "the box got through");
        assert!(worst < 0.1, "a ball is {worst:.3} m inside the box");
        assert!(w.vel.iter().any(|v| v.length() > 4.0), "balls were pushed along");
    }

    #[test]
    fn mixed_scene_does_not_depend_on_thread_count() {
        let mut outputs = Vec::new();
        for workers in [0, 3, 7] {
            let jobs = JobSystem::new(workers);
            let mut w = pile(800);
            w.add_static(StaticBox::new(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.3, 1.0)).with_rot(Quat::from_axis_angle(Vec3::X, 0.4)));
            for k in 0..4 {
                w.add_rigid(RigidBody::new_box(vec3(-1.5 + k as f32, 6.0 + k as f32, 0.5), Vec3::splat(0.3), 5.0));
            }
            let a = w.add_particle(vec3(2.0, 8.0, 2.0), 0.1, 0.0, 9).0;
            for k in 1..8 {
                w.add_particle(vec3(2.0 + 0.25 * k as f32, 8.0, 2.0), 0.1, 1.0, 9);
                w.add_constraint(DistanceConstraint::rod(a + k - 1, a + k, 0.25));
            }
            for _ in 0..90 {
                w.step(&jobs, 1.0 / 60.0);
            }
            let rigid: Vec<(Vec3, Quat)> = w.rigids.iter().map(|b| (b.pos, b.rot)).collect();
            outputs.push((w.pos, rigid));
        }
        assert!(outputs.windows(2).all(|o| o[0] == o[1]));
    }
}
