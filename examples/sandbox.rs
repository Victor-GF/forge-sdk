//! "sandbox": a test scene where every physics feature is driven by a game system.
//!
//! - raycast vehicle (rigid chassis, suspension and tires from 4 raycasts), with a roof cannon
//! - jump ramp and two walled zones you can jump into: a ball pit and a pool with buoyancy
//! - crate pyramid and a wrecking ball on a tethered chain
//! - a curtain of hanging ropes across the road, and a trampoline net under a ball fountain
//! - grenades: a raycast from the camera picks the spot, then a radial impulse
//!
//!   cargo run --release --example sandbox
//!   cargo run --release --example sandbox -- --headless 600   (scripted drive, writes out/sandbox.png)
//!
//!   cargo run --release --example sandbox -- --headless 600 --view 2 --shots 60   (fixed camera,
//!   one PNG per second)
//!
//!   cargo run --release --example sandbox -- --telemetry 120   (record 120 frames at 1, 2, 4, 8
//!   threads; prints scaling/cache/memory tables and writes out/telemetry.json for Perfetto;
//!   add --no-render to measure the simulation alone)
//!
//! Keys: W/S throttle/reverse, A/D steer, Space handbrake, F fire, G grenade at screen center,
//! B swing the wrecking ball, R reset car, N +300 balls in the pit, C cycles cameras (chase, then
//! fixed views of each zone), arrows orbit, Q/E zoom, P writes out/trace.json, Esc quits.

use std::{path::Path, time::Instant};

use forja::{
    image,
    math::Rng,
    physics::{
        self, DistanceConstraint, RayFilter, RigidBody, StaticBox,
        vehicle::{RaycastVehicle, VehicleInput},
        water::WaterVolume,
    },
    platform::{Input, Window},
    prelude::*,
    profile::{self, Report},
    scene::Sun,
};

const HALF: f32 = 40.0;
const BALL_DENSITY: f32 = 400.0;
const CAR_START: Vec3 = vec3(0.0, 1.3, 22.0);

// --- resources ---------------------------------------------------------------------------------

#[derive(Default)]
struct Controls(Input);

/// What the player (or the autopilot) wants. Written once per frame, read by tick systems.
/// One-shot actions stay set until a tick system consumes them, so a frame with zero ticks
/// doesn't lose a key press and a frame with two ticks doesn't fire twice.
#[derive(Default)]
struct Intent {
    drive: VehicleInput,
    fire: bool,
    grenade: bool,
    swing: bool,
    reset: bool,
    more_balls: bool,
}

struct Car(RaycastVehicle);

struct Meshes {
    sphere: MeshId,
}

struct CameraRig {
    yaw: f32,
    pitch: f32,
    dist: f32,
    eye: Vec3,
    target: Vec3,
    /// 0 = chase, otherwise `VIEWS[view - 1]`.
    view: usize,
}

struct Water(WaterVolume);

struct WreckingBall(BodyId);

struct Spawners {
    rng: Rng,
    pit_remaining: u32,
    fountain_remaining: u32,
    shots: u32,
}

struct Autopilot {
    enabled: bool,
    time: f32,
}

// --- components --------------------------------------------------------------------------------

/// Base color, so `heat` can tint by speed and fade back.
#[derive(Clone, Copy)]
struct Tint(Vec3);

/// Car parts drawn from the vehicle state instead of a physics body.
#[derive(Clone, Copy)]
enum CarPart {
    Wheel(usize),
    /// Fixed offset from the chassis.
    Attached(Vec3),
}

// --- frame systems -----------------------------------------------------------------------------

fn read_input(ctx: &SystemCtx) {
    let input = &ctx.res::<Controls>().0;
    let mut intent = ctx.res_mut::<Intent>();
    if ctx.res::<Autopilot>().enabled {
        return;
    }
    let speed = ctx.res::<Car>().0.forward_speed(&ctx.res::<PhysicsWorld>());
    let axis = |neg: u8, pos: u8| input.is_down(pos) as i32 as f32 - input.is_down(neg) as i32 as f32;
    let pedal = axis(b'S', b'W');
    // S brakes while rolling forward, then reverses.
    let (throttle, brake) = if pedal < 0.0 && speed > 1.0 { (0.0, 1.0) } else { (pedal, 0.0) };
    intent.drive = VehicleInput {
        throttle,
        steer: axis(b'A', b'D'),
        brake: if input.is_down(Input::SPACE) { 1.0 } else { brake },
    };
    intent.fire |= input.was_pressed(b'F');
    intent.grenade |= input.was_pressed(b'G');
    intent.swing |= input.was_pressed(b'B');
    intent.reset |= input.was_pressed(b'R');
    intent.more_balls |= input.was_pressed(b'N');
}

/// Chase camera with an orbit offset. Also centers the shadow map on the car.
fn chase_camera(ctx: &SystemCtx) {
    let input = &ctx.res::<Controls>().0;
    let mut rig = ctx.res_mut::<CameraRig>();
    let dt = ctx.frame.dt.min(0.1);
    if input.is_down(Input::LEFT) {
        rig.yaw -= 1.5 * dt;
    }
    if input.is_down(Input::RIGHT) {
        rig.yaw += 1.5 * dt;
    }
    if input.is_down(Input::UP) {
        rig.pitch = (rig.pitch + dt).min(1.3);
    }
    if input.is_down(Input::DOWN) {
        rig.pitch = (rig.pitch - dt).max(0.05);
    }
    if input.is_down(b'Q') {
        rig.dist = (rig.dist - 10.0 * dt).max(4.0);
    }
    if input.is_down(b'E') {
        rig.dist = (rig.dist + 10.0 * dt).min(40.0);
    }
    if input.was_pressed(b'C') {
        rig.view = (rig.view + 1) % (VIEWS.len() + 1);
    }
    let mut cam = ctx.res_mut::<Camera>();
    let mut sun = ctx.res_mut::<Sun>();
    if rig.view > 0 {
        let (eye, target, radius) = VIEWS[rig.view - 1];
        (cam.eye, cam.target, sun.shadow_center, sun.shadow_radius) = (eye, target, target, radius);
        return;
    }
    let phys = ctx.res::<PhysicsWorld>();
    let body = phys.rigid(ctx.res::<Car>().0.body);
    let fwd = body.rot.rotate(-Vec3::Z);
    let heading = fwd.x.atan2(-fwd.z) + rig.yaw;
    let back = vec3(-heading.sin(), 0.0, heading.cos());
    let target = body.pos + Vec3::Y;
    let eye = target + (back * rig.pitch.cos() + Vec3::Y * rig.pitch.sin()) * rig.dist;
    // Exponential smoothing, frame-rate independent.
    let k = 1.0 - (-6.0 * dt).exp();
    rig.eye = rig.eye.lerp(eye.max(vec3(-HALF, 0.5, -HALF)).min(vec3(HALF, 60.0, HALF)), k);
    rig.target = rig.target.lerp(target, k);
    (cam.eye, cam.target) = (rig.eye, rig.target);
    (sun.shadow_center, sun.shadow_radius) = (target, 22.0);
}

/// Fixed cameras (key C cycles chase -> these): (eye, target, shadow radius).
const VIEWS: [(Vec3, Vec3, f32); 5] = [
    (vec3(0.0, 48.0, 52.0), vec3(0.0, 0.0, -4.0), 45.0),
    (vec3(-4.0, 6.0, 12.0), vec3(-12.0, 2.5, 0.5), 12.0),
    (vec3(4.0, 3.5, 20.0), vec3(0.0, 2.5, 12.0), 10.0),
    (vec3(22.0, 7.0, 21.0), vec3(14.0, 2.5, 12.0), 10.0),
    (vec3(-8.0, 9.0, -2.0), vec3(-18.0, 2.0, -18.0), 14.0),
];

// --- tick systems ------------------------------------------------------------------------------

/// Scripted input for headless runs: drive through the ropes and over the ramp, then circle.
fn autopilot(ctx: &SystemCtx) {
    let mut auto = ctx.res_mut::<Autopilot>();
    auto.time += ctx.frame.dt;
    let t = auto.time;
    let mut intent = ctx.res_mut::<Intent>();
    let steer = if t < 5.0 { 0.0 } else { 0.45 };
    intent.drive = VehicleInput { throttle: 0.75, steer, brake: 0.0 };
    let every = |period: f32| (t / period).floor() != ((t - ctx.frame.dt) / period).floor();
    intent.fire |= t > 6.0 && every(1.0);
    intent.swing |= every(4.0);
    intent.grenade |= t > 3.0 && every(3.0);
}

fn drive_vehicle(ctx: &SystemCtx) {
    let mut intent = ctx.res_mut::<Intent>();
    let mut phys = ctx.res_mut::<PhysicsWorld>();
    let car = &mut ctx.res_mut::<Car>().0;
    if std::mem::take(&mut intent.reset) {
        let p = phys.rigid(car.body).pos;
        car.reset(&mut phys, vec3(p.x, p.y.max(0.0) + 1.5, p.z), 0.0);
    }
    car.update(&mut phys, intent.drive, ctx.frame.dt);
}

fn buoyancy(ctx: &SystemCtx) {
    ctx.res::<Water>().0.apply(&mut ctx.res_mut::<PhysicsWorld>(), ctx.jobs, ctx.frame.dt);
}

/// Roof cannon: a heavy ball leaves the barrel and the car takes the recoil.
fn cannon(ctx: &SystemCtx) {
    if !std::mem::take(&mut ctx.res_mut::<Intent>().fire) {
        return;
    }
    let mut phys = ctx.res_mut::<PhysicsWorld>();
    let car = &ctx.res::<Car>().0;
    let body = phys.rigid(car.body).clone();
    let fwd = body.rot.rotate(-Vec3::Z);
    let muzzle = body.to_world(vec3(0.0, 0.9, -1.6));
    let (r, mass) = (0.3, 120.0);
    let velocity = body.vel + fwd * 30.0 + body.rot.rotate(Vec3::Y) * 3.0;
    let id = phys.add_body(muzzle, r, mass);
    phys.vel[id.0 as usize] = velocity;
    phys.rigid_mut(car.body).apply_impulse_at((body.vel - velocity) * mass, muzzle);
    ctx.res_mut::<Spawners>().shots += 1;
    let color = vec3(0.08, 0.08, 0.09);
    ctx.commands.spawn((
        Transform::from_pos(muzzle).with_scale(Vec3::splat(r)),
        Body(id),
        Renderable { mesh: ctx.res::<Meshes>().sphere, color },
        Tint(color),
    ));
}

/// Picks the point under the screen center with a raycast and blows it up.
fn grenade(ctx: &SystemCtx) {
    if !std::mem::take(&mut ctx.res_mut::<Intent>().grenade) {
        return;
    }
    let cam = *ctx.res::<Camera>();
    let mut phys = ctx.res_mut::<PhysicsWorld>();
    let dir = (cam.target - cam.eye).normalize();
    let filter = RayFilter::ignoring(ctx.res::<Car>().0.body);
    if let Some(hit) = phys.raycast(cam.eye, dir, 200.0, filter) {
        phys.explode(hit.point, 6.0, 1500.0);
    }
}

fn wrecking_ball(ctx: &SystemCtx) {
    if !std::mem::take(&mut ctx.res_mut::<Intent>().swing) {
        return;
    }
    let ball = ctx.res::<WreckingBall>().0;
    let mut phys = ctx.res_mut::<PhysicsWorld>();
    let i = ball.0 as usize;
    phys.vel[i] = vec3(0.0, 0.0, -13.0);
}

/// Rain into the ball pit, and a fountain over the trampoline.
fn spawners(ctx: &SystemCtx) {
    let mut sp = ctx.res_mut::<Spawners>();
    if std::mem::take(&mut ctx.res_mut::<Intent>().more_balls) {
        sp.pit_remaining += 300;
    }
    let mut phys = ctx.res_mut::<PhysicsWorld>();
    let mesh = ctx.res::<Meshes>().sphere;
    let palette = [vec3(0.85, 0.25, 0.15), vec3(0.95, 0.65, 0.10), vec3(0.15, 0.55, 0.85), vec3(0.30, 0.75, 0.35), vec3(0.85, 0.85, 0.80)];
    let spawn = |sp: &mut Spawners, phys: &mut PhysicsWorld, p: Vec3| {
        let r = sp.rng.range(0.18, 0.3);
        let id = phys.add_body(p, r, ball_mass(r));
        let color = palette[(sp.rng.next_u64() % palette.len() as u64) as usize];
        ctx.commands.spawn((Transform::from_pos(p).with_scale(Vec3::splat(r)), Body(id), Renderable { mesh, color }, Tint(color)));
    };
    for _ in 0..sp.pit_remaining.min(20) {
        sp.pit_remaining -= 1;
        let p = vec3(sp.rng.range(11.0, 25.0), sp.rng.range(4.0, 8.0), sp.rng.range(-25.0, -11.0));
        spawn(&mut sp, &mut phys, p);
    }
    if sp.fountain_remaining > 0 && ctx.frame.frame.is_multiple_of(3) {
        sp.fountain_remaining -= 1;
        let p = vec3(sp.rng.range(12.0, 16.0), 9.0, sp.rng.range(10.0, 14.0));
        spawn(&mut sp, &mut phys, p);
    }
}

/// Wheels follow the suspension, steering and rolling; other parts ride on the chassis.
fn car_visuals(ctx: &SystemCtx) {
    let phys = ctx.res::<PhysicsWorld>();
    let car = &ctx.res::<Car>().0;
    let body = phys.rigid(car.body);
    for table in ctx.query::<(Transform, CarPart)>() {
        let mut transforms = table.write::<Transform>();
        for (t, part) in transforms.iter_mut().zip(table.read::<CarPart>().iter()) {
            match *part {
                CarPart::Wheel(i) => (t.pos, t.rot) = car.wheel_pose(&phys, i),
                CarPart::Attached(offset) => (t.pos, t.rot) = (body.to_world(offset), body.rot),
            }
        }
    }
}

/// Fast things glow hot. Runs in parallel with `physics.sync`: different columns.
fn heat(ctx: &SystemCtx) {
    let phys = ctx.res::<PhysicsWorld>();
    let vel = &phys.vel;
    let hot = vec3(1.6, 1.2, 0.6);
    for table in ctx.query::<(Body, Tint, Renderable)>() {
        let mut render = table.write::<Renderable>();
        let (bodies, tints) = (table.read::<Body>(), table.read::<Tint>());
        ctx.jobs.par_zip((&mut *render, &*bodies, &*tints), 1024, |_, (render, bodies, tints)| {
            for i in 0..render.len() {
                let speed = vel[bodies[i].0.0 as usize].length();
                render[i].color = tints[i].0.lerp(hot, (speed / 14.0).min(1.0));
            }
        });
    }
}

// --- scene -------------------------------------------------------------------------------------

fn ball_mass(r: f32) -> f32 {
    BALL_DENSITY * 4.0 / 3.0 * std::f32::consts::PI * r * r * r
}

struct Builder<'a> {
    world: &'a mut World,
    phys: PhysicsWorld,
    sphere: MeshId,
    cube: MeshId,
    next_group: u32,
}

impl Builder<'_> {
    /// Static box: collides and is drawn.
    fn wall(&mut self, center: Vec3, half: Vec3, rot: Quat, color: Vec3) {
        self.phys.add_static(StaticBox::new(center, half).with_rot(rot));
        self.world.spawn((Transform::from_pos(center).with_scale(half).with_rot(rot), Renderable { mesh: self.cube, color }));
    }

    fn crate_(&mut self, pos: Vec3, half: f32, density: f32, color: Vec3) -> RigidId {
        let h = Vec3::splat(half);
        let id = self.phys.add_rigid(RigidBody::new_box(pos, h, density * 8.0 * half * half * half));
        self.world.spawn((Transform::from_pos(pos).with_scale(h), Rigid(id), Renderable { mesh: self.cube, color }));
        id
    }

    fn particle(&mut self, pos: Vec3, r: f32, mass: f32, group: u32, color: Vec3) -> BodyId {
        let id = self.phys.add_particle(pos, r, mass, group);
        self.world.spawn((Transform::from_pos(pos).with_scale(Vec3::splat(r)), Body(id), Renderable { mesh: self.sphere, color }, Tint(color)));
        id
    }

    /// Chain of `links` particles hanging from a static anchor, rods between neighbours and
    /// tethers to the anchor. `link` and `end` are (radius, mass). Returns the last particle.
    fn rope(&mut self, anchor: Vec3, dir: Vec3, links: u32, spacing: f32, link: (f32, f32), end: (f32, f32)) -> BodyId {
        let group = self.next_group;
        self.next_group += 1;
        let color = vec3(0.75, 0.62, 0.40);
        let first = self.particle(anchor, link.0, 0.0, group, color).0;
        for k in 1..=links {
            let ((r, mass), color) = if k == links { (end, vec3(0.2, 0.2, 0.22)) } else { (link, color) };
            self.particle(anchor + dir * (k as f32 * spacing), r, mass, group, color);
            self.phys.add_constraint(DistanceConstraint::rod(first + k - 1, first + k, spacing));
            self.phys.add_constraint(DistanceConstraint::tether(first, first + k, spacing * k as f32));
        }
        BodyId(first + links)
    }

    /// Square net of `n` x `n` knots with a pinned border.
    fn net(&mut self, center: Vec3, n: u32, spacing: f32) {
        let group = self.next_group;
        self.next_group += 1;
        let first = self.phys.len() as u32;
        let offset = (n - 1) as f32 * spacing * 0.5;
        for z in 0..n {
            for x in 0..n {
                let border = x == 0 || z == 0 || x == n - 1 || z == n - 1;
                let p = center + vec3(x as f32 * spacing - offset, 0.0, z as f32 * spacing - offset);
                let color = if border { vec3(0.3, 0.3, 0.32) } else { vec3(0.2, 0.75, 0.7) };
                self.particle(p, 0.15, if border { 0.0 } else { 1.5 }, group, color);
            }
        }
        let at = |x: u32, z: u32| first + z * n + x;
        let compliance = 2e-5;
        let diag = spacing * std::f32::consts::SQRT_2;
        for z in 0..n {
            for x in 0..n {
                if x + 1 < n {
                    self.phys.add_constraint(DistanceConstraint::spring(at(x, z), at(x + 1, z), spacing, compliance));
                }
                if z + 1 < n {
                    self.phys.add_constraint(DistanceConstraint::spring(at(x, z), at(x, z + 1), spacing, compliance));
                }
                if x + 1 < n && z + 1 < n {
                    self.phys.add_constraint(DistanceConstraint::spring(at(x, z), at(x + 1, z + 1), diag, compliance * 4.0));
                    self.phys.add_constraint(DistanceConstraint::spring(at(x + 1, z), at(x, z + 1), diag, compliance * 4.0));
                }
            }
        }
    }
}

fn build(world: &mut World, sphere: MeshId, cube: MeshId, wheel: MeshId, autopilot: bool, view: usize) {
    let mut phys = PhysicsWorld::default();
    phys.bounds_min = vec3(-HALF, 0.0, -HALF);
    phys.bounds_max = vec3(HALF, f32::INFINITY, HALF);
    let mut b = Builder { world, phys, sphere, cube, next_group: 1 };
    let (concrete, wood, steel) = (vec3(0.55, 0.52, 0.48), vec3(0.62, 0.45, 0.28), vec3(0.35, 0.37, 0.40));
    let flat = Quat::IDENTITY;

    b.world.spawn((Transform::from_pos(vec3(0.0, -0.5, 0.0)).with_scale(vec3(HALF, 0.5, HALF)), Renderable { mesh: cube, color: vec3(0.42, 0.44, 0.38) }));
    for (pos, half) in [
        (vec3(HALF + 0.3, 0.8, 0.0), vec3(0.3, 0.8, HALF)),
        (vec3(-HALF - 0.3, 0.8, 0.0), vec3(0.3, 0.8, HALF)),
        (vec3(0.0, 0.8, HALF + 0.3), vec3(HALF, 0.8, 0.3)),
        (vec3(0.0, 0.8, -HALF - 0.3), vec3(HALF, 0.8, 0.3)),
    ] {
        b.world.spawn((Transform::from_pos(pos).with_scale(half), Renderable { mesh: cube, color: concrete }));
    }

    // Jump ramp on the road: rises towards -Z.
    let ramp = |angle: f32, length: f32| {
        let rot = Quat::from_axis_angle(Vec3::X, angle);
        let half = vec3(2.5, 0.2, length);
        // Back edge flush with the ground.
        let y = length * angle.sin() - 0.2 * angle.cos();
        (rot, half, y)
    };
    let (rot, half, y) = ramp(0.22, 4.0);
    b.wall(vec3(0.0, y, 4.0), half, rot, concrete);

    // Ball pit (x 10..26, z -26..-10) with an entry ramp over its +Z wall.
    let (lo, hi, h) = (vec3(10.0, 0.0, -26.0), vec3(26.0, 0.0, -10.0), 0.6);
    let pit_color = vec3(0.70, 0.45, 0.25);
    b.wall(vec3((lo.x + hi.x) / 2.0, h, lo.z), vec3((hi.x - lo.x) / 2.0 + 0.3, h, 0.3), flat, pit_color);
    b.wall(vec3((lo.x + hi.x) / 2.0, h, hi.z), vec3((hi.x - lo.x) / 2.0 + 0.3, h, 0.3), flat, pit_color);
    b.wall(vec3(lo.x, h, (lo.z + hi.z) / 2.0), vec3(0.3, h, (hi.z - lo.z) / 2.0), flat, pit_color);
    b.wall(vec3(hi.x, h, (lo.z + hi.z) / 2.0), vec3(0.3, h, (hi.z - lo.z) / 2.0), flat, pit_color);
    let (rot, half, y) = ramp(0.2, 4.0);
    b.wall(vec3(18.0, y, hi.z + 4.4), half, rot, concrete);

    // Pool (x -26..-10, z -26..-10), water 2.2 m deep, walls 2.6 m, entry ramp over the +Z wall.
    let (lo, hi, h) = (vec3(-26.0, 0.0, -26.0), vec3(-10.0, 0.0, -10.0), 1.3);
    b.wall(vec3((lo.x + hi.x) / 2.0, h, lo.z), vec3((hi.x - lo.x) / 2.0 + 0.3, h, 0.3), flat, concrete);
    b.wall(vec3((lo.x + hi.x) / 2.0, h, hi.z), vec3((hi.x - lo.x) / 2.0 + 0.3, h, 0.3), flat, concrete);
    b.wall(vec3(lo.x, h, (lo.z + hi.z) / 2.0), vec3(0.3, h, (hi.z - lo.z) / 2.0), flat, concrete);
    b.wall(vec3(hi.x, h, (lo.z + hi.z) / 2.0), vec3(0.3, h, (hi.z - lo.z) / 2.0), flat, concrete);
    let (rot, half, y) = ramp(0.33, 8.0);
    b.wall(vec3(-18.0, y, hi.z + 8.2), half, rot, concrete);
    let water = WaterVolume { min: vec3(lo.x, 0.0, lo.z), max: vec3(hi.x, 2.2, hi.z), density: 1000.0, drag: 1.5 };
    let center = (water.min + water.max) * 0.5;
    b.world.spawn((Transform::from_pos(center).with_scale((water.max - water.min) * 0.5), Renderable { mesh: cube, color: vec3(0.10, 0.30, 0.55) }));
    let mut rng = Rng::new(42);
    for _ in 0..60 {
        let r = rng.range(0.2, 0.35);
        let p = vec3(rng.range(-24.0, -12.0), rng.range(2.5, 4.0), rng.range(-24.0, -12.0));
        b.particle(p, r, ball_mass(r), 0, vec3(0.95, 0.85, 0.2));
    }
    for k in 0..5 {
        b.crate_(vec3(-22.0 + k as f32 * 2.5, 3.0, -18.0 + (k % 2) as f32 * 3.0), 0.45, 300.0, wood);
    }

    // Crate pyramid and a wrecking ball hanging from a gantry in front of it.
    let base = vec3(-12.0, 0.0, 0.0);
    for row in 0..5 {
        for k in 0..5 - row {
            let x = (k as f32 - (4 - row) as f32 * 0.5) * 0.92;
            b.crate_(base + vec3(x, 0.45 + row as f32 * 0.9, 0.0), 0.45, 250.0, wood);
        }
    }
    let anchor = base + vec3(0.0, 10.0, 1.8);
    b.wall(anchor + vec3(-4.0, -5.0, 0.0), vec3(0.2, 5.2, 0.2), flat, steel);
    b.wall(anchor + vec3(4.0, -5.0, 0.0), vec3(0.2, 5.2, 0.2), flat, steel);
    b.wall(anchor + vec3(0.0, 0.35, 0.0), vec3(4.2, 0.2, 0.2), flat, steel);
    let ball = b.rope(anchor, -Vec3::Y, 18, 0.4, (0.1, 10.0), (0.5, 900.0));

    // Curtain of ropes across the road, before the ramp.
    let beam = vec3(0.0, 5.2, 13.0);
    b.wall(beam + vec3(-4.2, -2.6, 0.0), vec3(0.15, 2.6, 0.15), flat, steel);
    b.wall(beam + vec3(4.2, -2.6, 0.0), vec3(0.15, 2.6, 0.15), flat, steel);
    b.wall(beam + vec3(0.0, 0.3, 0.0), vec3(4.4, 0.15, 0.15), flat, steel);
    for k in 0..8 {
        b.rope(beam + vec3(-3.5 + k as f32, 0.0, 0.0), -Vec3::Y, 12, 0.32, (0.1, 2.0), (0.22, 15.0));
    }

    // Trampoline under a ball fountain, on four posts.
    let net_center = vec3(14.0, 3.0, 12.0);
    let (n, spacing) = (22, 0.3);
    let corner = (n - 1) as f32 * spacing * 0.5;
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let p = net_center + vec3(sx * (corner + 0.35), -1.5, sz * (corner + 0.35));
        b.wall(p, vec3(0.15, 1.5, 0.15), flat, steel);
    }
    b.net(net_center, n, spacing);

    // A few loose crates near the start.
    for k in 0..4 {
        b.crate_(vec3(6.0 + k as f32 * 1.1, 0.4, 18.0 - k as f32 * 0.3), 0.4, 250.0, wood);
    }

    // The car: chassis box, cabin, cannon barrel, four wheels.
    let half = vec3(0.9, 0.3, 2.0);
    let chassis = b.phys.add_rigid(RigidBody::new_box(CAR_START, half, 1200.0));
    let car = RaycastVehicle::four_wheels(chassis, half, 0.4);
    let red = vec3(0.8, 0.12, 0.1);
    b.world.spawn((Transform::from_pos(CAR_START).with_scale(half), Rigid(chassis), Renderable { mesh: cube, color: red }));
    b.world.spawn((Transform::default().with_scale(vec3(0.75, 0.3, 0.9)), CarPart::Attached(vec3(0.0, 0.55, 0.3)), Renderable { mesh: cube, color: vec3(0.85, 0.85, 0.9) }));
    b.world.spawn((Transform::default().with_scale(vec3(0.12, 0.12, 0.7)), CarPart::Attached(vec3(0.0, 0.9, -0.9)), Renderable { mesh: cube, color: steel }));
    for (i, w) in car.wheels.iter().enumerate() {
        b.world.spawn((Transform::default().with_scale(vec3(0.15, w.radius, w.radius)), CarPart::Wheel(i), Renderable { mesh: wheel, color: vec3(0.1, 0.1, 0.1) }));
    }

    let phys = b.phys;
    world.insert_resource(phys);
    world.insert_resource(Car(car));
    world.insert_resource(Water(water));
    world.insert_resource(WreckingBall(ball));
    world.insert_resource(Meshes { sphere });
    world.insert_resource(Spawners { rng: Rng::new(0x5eed), pit_remaining: 900, fountain_remaining: 250, shots: 0 });
    world.insert_resource(Intent::default());
    world.insert_resource(Controls::default());
    world.insert_resource(Autopilot { enabled: autopilot, time: 0.0 });
    world.insert_resource(CameraRig { yaw: 0.35, pitch: 0.35, dist: 11.0, eye: CAR_START + vec3(3.0, 5.0, 10.0), target: CAR_START, view });
    world.insert_resource(Camera { fov_y: 1.0, far: 300.0, ..Camera::default() });
    world.insert_resource(Sun { shadow_radius: 22.0, ..Sun::default() });
}

// --- main --------------------------------------------------------------------------------------

#[global_allocator]
static ALLOC: forja::alloc::CountingAlloc = forja::alloc::CountingAlloc;

struct Args {
    headless: Option<u64>,
    threads: Option<usize>,
    width: usize,
    height: usize,
    view: usize,
    /// Headless: also write a PNG every this many frames.
    shots: Option<u64>,
    /// Record this many frames per thread count (after a warm-up) and write out/telemetry.json.
    telemetry: Option<u64>,
    /// Simulation only: skip rendering (to measure physics without the render competing).
    no_render: bool,
}

fn parse_args() -> Args {
    let mut a = Args { headless: None, threads: None, width: 960, height: 540, view: 0, shots: None, telemetry: None, no_render: false };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize| -> u64 { args.get(i + 1).and_then(|v| v.parse().ok()).expect("expected a number after the flag") };
    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--headless" => a.headless = Some(num(i)),
            "--threads" => a.threads = Some(num(i) as usize),
            "--width" => a.width = num(i) as usize,
            "--height" => a.height = num(i) as usize,
            "--view" => a.view = num(i) as usize,
            "--shots" => a.shots = Some(num(i)),
            "--telemetry" => a.telemetry = Some(num(i)),
            "--no-render" => a.no_render = true,
            _ => {}
        }
    }
    a
}

/// The scene with its schedules, render pipeline and clock: one `frame` call per frame.
struct Sandbox {
    world: World,
    frame_schedule: Schedule,
    tick_schedule: Schedule,
    pipeline: FramePipeline<SoftwareRenderer>,
    clock: FrameClock,
    render: bool,
}

impl Sandbox {
    fn new(args: &Args, headless: bool) -> Sandbox {
        let mut renderer = SoftwareRenderer::new(args.width, args.height);
        let sphere = renderer.add_mesh_lods(vec![(Mesh::sphere(20, 12), 24.0), (Mesh::sphere(12, 8), 8.0), (Mesh::sphere(8, 5), 0.0)]);
        let cube = renderer.add_mesh(Mesh::cube());
        let wheel = renderer.add_mesh(Mesh::cylinder(18));
        renderer.settings.fog_density = 0.0025;

        let mut world = World::new();
        build(&mut world, sphere, cube, wheel, headless, args.view);

        let mut frame_schedule = Schedule::new("frame");
        frame_schedule
            .add(system("input", read_input).reads::<Controls>().reads::<Autopilot>().reads::<Car>().reads::<PhysicsWorld>().writes::<Intent>())
            .add(
                system("camera", chase_camera)
                    .reads::<Controls>()
                    .reads::<Car>()
                    .reads::<PhysicsWorld>()
                    .writes::<CameraRig>()
                    .writes::<Camera>()
                    .writes::<Sun>(),
            );

        let mut tick_schedule = Schedule::new("tick");
        if headless {
            tick_schedule.add(system("autopilot", autopilot).writes::<Autopilot>().writes::<Intent>());
        }
        tick_schedule
            .add(system("vehicle.drive", drive_vehicle).writes::<Intent>().writes::<Car>().writes::<PhysicsWorld>())
            .add(system("water.buoyancy", buoyancy).reads::<Water>().writes::<PhysicsWorld>())
            .add(system("cannon", cannon).reads::<Car>().reads::<Meshes>().writes::<Intent>().writes::<Spawners>().writes::<PhysicsWorld>())
            .add(system("grenade", grenade).reads::<Camera>().reads::<Car>().writes::<Intent>().writes::<PhysicsWorld>())
            .add(system("wrecking_ball", wrecking_ball).reads::<WreckingBall>().writes::<Intent>().writes::<PhysicsWorld>())
            .add(system("spawners", spawners).reads::<Meshes>().writes::<Intent>().writes::<Spawners>().writes::<PhysicsWorld>())
            .add_all(physics::systems())
            .add(system("car.visuals", car_visuals).reads::<PhysicsWorld>().reads::<Car>().reads::<CarPart>().writes::<Transform>())
            .add(system("heat", heat).reads::<PhysicsWorld>().reads::<Body>().reads::<Tint>().writes::<Renderable>());

        let clock = if headless { FrameClock::deterministic(60.0) } else { FrameClock::new(60.0) };
        Sandbox { world, frame_schedule, tick_schedule, pipeline: FramePipeline::new(renderer), clock, render: !args.no_render }
    }

    /// Runs one frame; `present` gets the renderer once the previous frame is drawn. Returns the
    /// frame number and how many simulation ticks it ran.
    fn frame(&mut self, jobs: &Jobs, input: &Input, present: impl FnOnce(&mut SoftwareRenderer)) -> (u64, u32) {
        self.world.resource_mut::<Controls>().0 = input.clone();
        let frame = self.clock.begin();
        self.frame_schedule.run(&mut self.world, jobs, &frame);
        let mut ticks = 0;
        for tick in frame.tick_infos() {
            self.tick_schedule.run(&mut self.world, jobs, &tick);
            ticks += 1;
        }
        if self.render {
            self.pipeline.submit(&self.world, jobs, &frame, present);
        }
        (frame.frame, ticks)
    }

    /// Heap bytes of the simulation side (the renderer reports its own in `present`).
    fn memory(&self) -> Vec<(&'static str, usize)> {
        let mut m = self.world.memory();
        m.extend(self.world.res::<PhysicsWorld>().memory());
        m
    }
}

fn main() {
    let args = parse_args();
    if let Some(frames) = args.telemetry {
        telemetry(&args, frames);
        return;
    }
    profile::set_enabled(true);
    let jobs = match args.threads {
        Some(n) => JobSystem::new(n.max(1) - 1),
        None => JobSystem::with_available_parallelism(),
    };
    let mut sandbox = Sandbox::new(&args, args.headless.is_some());
    print!("{}{}", sandbox.frame_schedule.describe(), sandbox.tick_schedule.describe());
    println!("{} threads", jobs.thread_count());

    let mut window = match args.headless {
        Some(_) => None,
        None => Some(Window::open("forja — sandbox", args.width, args.height).expect("could not open window")),
    };
    let mut input = Input::default();
    let mut last_events = Vec::new();
    let (mut window_start, mut window_frames) = (Instant::now(), 0u32);
    let run_start = Instant::now();
    let mut frames = 0u64;
    let _ = std::fs::create_dir_all("out");

    loop {
        match (&mut window, args.headless) {
            (Some(w), _) => {
                if !w.pump(&mut input) || input.is_down(Input::ESCAPE) {
                    break;
                }
            }
            (None, Some(n)) if frames >= n => break,
            _ => {}
        }
        sandbox.frame(&jobs, &input, |r| {
            if let Some(w) = &mut window {
                w.present(&r.color, r.width, r.height);
            }
            // The renderer presents the previous frame.
            if let Some(every) = args.shots
                && frames > 0
                && frames.is_multiple_of(every)
            {
                let path = format!("out/sandbox_{:04}.png", frames);
                image::write_png(Path::new(&path), r.width, r.height, &r.color).expect("write png");
            }
        });
        frames += 1;
        window_frames += 1;

        if input.was_pressed(b'P') && !last_events.is_empty() {
            let _ = profile::write_chrome_trace(Path::new("out/trace.json"), &last_events);
            println!("wrote out/trace.json");
        }
        if window_frames == 60 {
            let elapsed = window_start.elapsed().as_secs_f64();
            last_events = profile::take_events();
            let _ = profile::take_counters();
            let report = Report::summarize(&last_events, jobs.thread_count());
            let phys = sandbox.world.res::<PhysicsWorld>();
            let speed = sandbox.world.res::<Car>().0.forward_speed(&phys) * 3.6;
            let line = format!(
                "{:5.1} fps {:5.2} ms/frame  util {:4.1}%  {:4.0} km/h  {} particles, {} boxes, {} constraints",
                window_frames as f64 / elapsed,
                elapsed * 1000.0 / window_frames as f64,
                report.utilization() * 100.0,
                speed,
                phys.len(),
                phys.rigids.len(),
                phys.constraints.len(),
            );
            match &mut window {
                Some(w) => w.set_title(&format!("forja — sandbox | {line}")),
                None => println!("frame {frames:5}: {line}"),
            }
            if args.headless.is_some() && frames.is_multiple_of(300) {
                report.print(window_frames);
            }
            (window_start, window_frames) = (Instant::now(), 0);
        }
    }

    let r = sandbox.pipeline.finish();
    println!(
        "{frames} frames in {:.2}s ({:.2} ms/frame); last frame: {} instances, {} triangles",
        run_start.elapsed().as_secs_f64(),
        run_start.elapsed().as_secs_f64() * 1000.0 / frames.max(1) as f64,
        r.stats.instances,
        r.stats.triangles,
    );
    if args.headless.is_some() {
        image::write_png(Path::new("out/sandbox.png"), r.width, r.height, &r.color).expect("write png");
        if !last_events.is_empty() {
            profile::write_chrome_trace(Path::new("out/trace.json"), &last_events).expect("write trace");
        }
        println!("wrote out/sandbox.png and out/trace.json");
    }
}

/// Same scripted scene at 1, 2, 4, ... threads: warm up, record `frames` frames, then write one
/// Perfetto trace with a process per thread count and print the summary tables.
fn telemetry(args: &Args, frames: u64) {
    use forja::telemetry::{self, Capture};
    const WARMUP: u64 = 240;
    let max = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut counts: Vec<usize> = [1, 2, 4, 8, 16, 32].into_iter().filter(|&t| t < max).collect();
    counts.push(max);
    let caches = forja::cpu::caches();
    println!("{} | {} | simd: {}", forja::cpu::brand(), forja::cpu::required(), forja::simd::IMPLEMENTATION);

    struct Recorded {
        capture: Capture,
        counters: Vec<profile::Counter>,
        spans: Vec<profile::Span>,
    }
    let mut recorded = Vec::new();
    let mut summaries = Vec::new();
    let input = Input::default();
    for &threads in &counts {
        let jobs = JobSystem::new(threads - 1);
        let mut sandbox = Sandbox::new(args, true);
        profile::set_enabled(false);
        for _ in 0..WARMUP {
            sandbox.frame(&jobs, &input, |_| {});
        }
        let _ = (profile::take_events(), profile::take_counters(), profile::take_flows());
        let jobs_before = jobs.jobs_run();
        let mut recorder = telemetry::Recorder::default();
        let mut render_memory = Vec::new();
        profile::set_enabled(true);
        for _ in 0..frames {
            recorder.begin_frame(&jobs);
            let (frame, ticks) = sandbox.frame(&jobs, &input, |r| render_memory = r.memory());
            recorder.end_frame(&jobs, frame, ticks);
            let mut memory = sandbox.memory();
            memory.extend(render_memory.iter().copied());
            recorder.sample_memory(&memory);
        }
        // Lets the last frame finish rendering, still recorded.
        sandbox.pipeline.finish();
        profile::set_enabled(false);
        let mut memory = sandbox.memory();
        memory.extend(render_memory.iter().copied());
        let capture = Capture {
            threads,
            events: profile::take_events(),
            counters: profile::take_counters(),
            flows: profile::take_flows(),
            frames: recorder.frames,
            jobs_per_thread: jobs.jobs_run().iter().zip(&jobs_before).map(|(a, b)| a - b).collect(),
            memory,
        };
        let lives = telemetry::frame_lives(&capture.events);
        let spans = telemetry::pipeline_spans(&lives);
        let mut counters = capture.counters.clone();
        counters.extend(telemetry::frame_counters(&capture.events, &capture.frames, &lives));
        counters.sort_by_key(|c| c.ts_ns);
        let summary = telemetry::summarize(&capture, &lives);
        println!(
            "  {threads:>2} threads: {:.2} ms/frame over {} frames, {} events, {} arrows",
            summary.macro_.frame_ms.mean,
            frames,
            capture.events.len(),
            capture.flows.len() / 2
        );
        summaries.push(summary);
        recorded.push(Recorded { capture, counters, spans });
    }

    let runs: Vec<profile::TraceRun> = recorded
        .iter()
        .map(|r| profile::TraceRun {
            pid: r.capture.threads as u32,
            name: format!("sandbox, {} thread{}", r.capture.threads, if r.capture.threads == 1 { "" } else { "s" }),
            events: &r.capture.events,
            counters: &r.counters,
            flows: &r.capture.flows,
            spans: &r.spans,
            thread_names: (0..r.capture.threads).map(profile::thread_name).collect(),
            frames: &r.capture.frames,
        })
        .collect();
    let _ = std::fs::create_dir_all("out");
    let path = if args.no_render { "out/telemetry_no_render.json" } else { "out/telemetry.json" };
    profile::write_trace(Path::new(path), &runs).expect("write trace");
    telemetry::print_report(&summaries, &caches);
    println!("\nwrote {path}, open it at https://ui.perfetto.dev");
}
