//! "rain": thousands of spheres rain into a box. Exercises the whole engine: frame model,
//! schedules with parallel systems, commands, physics, extract + pipelined shaded rendering.
//!
//!   cargo run --release --example rain
//!   cargo run --release --example rain -- --headless 600 --threads 8 --balls 4000
//!
//! Window controls: arrows/WASD orbit, Q/E zoom, Space +1000 balls, R toggles auto-rotate,
//! P writes out/trace.json (open in ui.perfetto.dev), Esc quits.

use std::{path::Path, time::Instant};

use forja::{
    image,
    math::Rng,
    physics,
    platform::{Input, Window},
    prelude::*,
    profile::{self, Report},
    scene::Sun,
};

const ARENA: f32 = 10.0;

/// Resource: this frame's keyboard state.
#[derive(Default)]
struct Controls(Input);

/// Resource: orbit camera parameters.
struct CameraRig {
    yaw: f32,
    pitch: f32,
    dist: f32,
    auto: bool,
}

/// Resource: how many balls are still to come.
struct Spawner {
    rng: Rng,
    remaining: u32,
    per_tick: u32,
    mesh: MeshId,
}

/// Component: base color, so `heat` can tint by speed and fade back.
#[derive(Clone, Copy)]
struct Tint(Vec3);

// --- systems ---------------------------------------------------------------------------------

fn handle_input(ctx: &SystemCtx) {
    let input = &ctx.res::<Controls>().0;
    if input.was_pressed(Input::SPACE) {
        ctx.res_mut::<Spawner>().remaining += 1000;
    }
}

fn orbit_camera(ctx: &SystemCtx) {
    let input = &ctx.res::<Controls>().0;
    let mut rig = ctx.res_mut::<CameraRig>();
    let dt = ctx.frame.dt;
    let key = |a: u8, b: u8| input.is_down(a) || input.is_down(b);
    if key(Input::LEFT, b'A') {
        rig.yaw -= 1.5 * dt;
    }
    if key(Input::RIGHT, b'D') {
        rig.yaw += 1.5 * dt;
    }
    if key(Input::UP, b'W') {
        rig.pitch = (rig.pitch + dt).min(1.4);
    }
    if key(Input::DOWN, b'S') {
        rig.pitch = (rig.pitch - dt).max(0.05);
    }
    if input.is_down(b'Q') {
        rig.dist = (rig.dist - 12.0 * dt).max(6.0);
    }
    if input.is_down(b'E') {
        rig.dist = (rig.dist + 12.0 * dt).min(60.0);
    }
    if input.was_pressed(b'R') {
        rig.auto = !rig.auto;
    }
    if rig.auto {
        rig.yaw += 0.15 * dt;
    }
    let target = vec3(0.0, 2.0, 0.0);
    let dir = vec3(rig.yaw.sin() * rig.pitch.cos(), rig.pitch.sin(), rig.yaw.cos() * rig.pitch.cos());
    let mut cam = ctx.res_mut::<Camera>();
    cam.eye = target + dir * rig.dist;
    cam.target = target;
}

fn spawn_rain(ctx: &SystemCtx) {
    let mut spawner = ctx.res_mut::<Spawner>();
    let n = spawner.remaining.min(spawner.per_tick);
    if n == 0 {
        return;
    }
    spawner.remaining -= n;
    let mut phys = ctx.res_mut::<PhysicsWorld>();
    let palette = [vec3(0.85, 0.25, 0.15), vec3(0.95, 0.65, 0.10), vec3(0.15, 0.55, 0.85), vec3(0.30, 0.75, 0.35), vec3(0.85, 0.85, 0.80)];
    for _ in 0..n {
        let rng = &mut spawner.rng;
        let r = rng.range(0.18, 0.32);
        let p = vec3(rng.range(-ARENA + 1.0, ARENA - 1.0), rng.range(12.0, 18.0), rng.range(-ARENA + 1.0, ARENA - 1.0));
        let color = palette[(rng.next_u64() % palette.len() as u64) as usize];
        let id = phys.add_body(p, r, r * r * r);
        phys.vel[id.0 as usize] = vec3(rng.range(-1.0, 1.0), rng.range(-2.0, 0.0), rng.range(-1.0, 1.0));
        ctx.commands.spawn((
            Transform::from_pos(p).with_scale(Vec3::splat(r)),
            Body(id),
            Renderable { mesh: spawner.mesh, color },
            Tint(color),
        ));
    }
}

/// Fast balls glow hot. Reads physics, writes colors: runs in parallel with `physics.sync`
/// (which writes transforms), because their declared writes don't overlap.
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
                render[i].color = tints[i].0.lerp(hot, (speed / 12.0).min(1.0));
            }
        });
    }
}

// --- setup -----------------------------------------------------------------------------------

fn setup(world: &mut World, sphere: MeshId, cube: MeshId, balls: u32) {
    let mut phys = PhysicsWorld::default();
    phys.bounds_min = vec3(-ARENA, 0.0, -ARENA);
    phys.bounds_max = vec3(ARENA, f32::INFINITY, ARENA);
    world.insert_resource(phys);
    world.insert_resource(Spawner { rng: Rng::new(0x5eed), remaining: balls, per_tick: 25, mesh: sphere });
    world.insert_resource(Controls::default());
    world.insert_resource(CameraRig { yaw: 0.6, pitch: 0.55, dist: 30.0, auto: true });
    world.insert_resource(Camera { fov_y: 0.9, ..Camera::default() });
    world.insert_resource(Sun { shadow_center: vec3(0.0, 2.0, 0.0), shadow_radius: 16.0, ..Sun::default() });

    let ground = vec3(0.42, 0.40, 0.38);
    world.spawn((Transform::from_pos(vec3(0.0, -0.5, 0.0)).with_scale(vec3(ARENA, 0.5, ARENA)), Renderable { mesh: cube, color: ground }));
    let wall = vec3(0.55, 0.50, 0.45);
    let (t, h) = (0.15, 0.6);
    for (pos, scale) in [
        (vec3(ARENA + t, h, 0.0), vec3(t, h, ARENA + 2.0 * t)),
        (vec3(-ARENA - t, h, 0.0), vec3(t, h, ARENA + 2.0 * t)),
        (vec3(0.0, h, ARENA + t), vec3(ARENA, h, t)),
        (vec3(0.0, h, -ARENA - t), vec3(ARENA, h, t)),
    ] {
        world.spawn((Transform::from_pos(pos).with_scale(scale), Renderable { mesh: cube, color: wall }));
    }
}

struct Args {
    headless: Option<u64>,
    threads: Option<usize>,
    balls: u32,
    width: usize,
    height: usize,
}

fn parse_args() -> Args {
    let mut a = Args { headless: None, threads: None, balls: 3000, width: 960, height: 540 };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize| -> u64 { args.get(i + 1).and_then(|v| v.parse().ok()).expect("expected a number after the flag") };
    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--headless" => a.headless = Some(num(i)),
            "--threads" => a.threads = Some(num(i) as usize),
            "--balls" => a.balls = num(i) as u32,
            "--width" => a.width = num(i) as usize,
            "--height" => a.height = num(i) as usize,
            _ => {}
        }
    }
    a
}

fn main() {
    let args = parse_args();
    profile::set_enabled(true);
    let jobs = match args.threads {
        Some(n) => JobSystem::new(n.max(1) - 1),
        None => JobSystem::with_available_parallelism(),
    };

    let mut renderer = SoftwareRenderer::new(args.width, args.height);
    // Balls are ~5 px on screen most of the time; pick detail by projected radius.
    let sphere = renderer.add_mesh_lods(vec![(Mesh::sphere(20, 12), 24.0), (Mesh::sphere(12, 8), 8.0), (Mesh::sphere(8, 5), 0.0)]);
    let cube = renderer.add_mesh(Mesh::cube());
    renderer.settings.fog_density = 0.004;

    let mut world = World::new();
    setup(&mut world, sphere, cube, args.balls);

    // Variable-rate work, once per frame.
    let mut frame_schedule = Schedule::new("frame");
    frame_schedule
        .add(system("input", handle_input).reads::<Controls>().writes::<Spawner>())
        .add(system("camera", orbit_camera).reads::<Controls>().writes::<CameraRig>().writes::<Camera>());

    // Fixed-rate simulation, 0..n times per frame.
    let mut tick_schedule = Schedule::new("tick");
    tick_schedule
        .add(system("spawn", spawn_rain).writes::<Spawner>().writes::<PhysicsWorld>())
        .add_all(physics::systems())
        .add(system("heat", heat).reads::<PhysicsWorld>().reads::<Body>().reads::<Tint>().writes::<Renderable>());

    print!("{}{}", frame_schedule.describe(), tick_schedule.describe());
    println!("{} threads", jobs.thread_count());

    let mut window = match args.headless {
        Some(_) => None,
        None => Some(Window::open("forja — rain", args.width, args.height).expect("could not open window")),
    };
    let mut clock = match args.headless {
        Some(_) => FrameClock::deterministic(60.0),
        None => FrameClock::new(60.0),
    };
    let mut pipeline = FramePipeline::new(renderer);
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
        world.resource_mut::<Controls>().0 = input.clone();

        let frame = clock.begin();
        frame_schedule.run(&mut world, &jobs, &frame);
        for tick in frame.tick_infos() {
            tick_schedule.run(&mut world, &jobs, &tick);
        }
        pipeline.submit(&world, &jobs, &frame, |r| {
            if let Some(w) = &mut window {
                w.present(&r.color, r.width, r.height);
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
            let report = Report::summarize(&last_events, jobs.thread_count());
            let balls = world.res::<PhysicsWorld>().len();
            let line = format!(
                "{:6.1} fps  {:5.2} ms/frame  utilization {:4.1}%  {} balls",
                window_frames as f64 / elapsed,
                elapsed * 1000.0 / window_frames as f64,
                report.utilization() * 100.0,
                balls
            );
            match &mut window {
                Some(w) => w.set_title(&format!("forja — rain | {line}")),
                None => {
                    println!("frame {frames:5}: {line}");
                }
            }
            if args.headless.is_some() && frames.is_multiple_of(300) {
                report.print(window_frames);
            }
            (window_start, window_frames) = (Instant::now(), 0);
        }
    }

    let r = pipeline.finish();
    println!(
        "{frames} frames in {:.2}s ({:.2} ms/frame); last frame: {} instances, {} triangles, {} binned",
        run_start.elapsed().as_secs_f64(),
        run_start.elapsed().as_secs_f64() * 1000.0 / frames.max(1) as f64,
        r.stats.instances,
        r.stats.triangles,
        r.stats.binned
    );
    if args.headless.is_some() {
        image::write_png(Path::new("out/rain.png"), r.width, r.height, &r.color).expect("write png");
        if !last_events.is_empty() {
            profile::write_chrome_trace(Path::new("out/trace.json"), &last_events).expect("write trace");
        }
        println!("wrote out/rain.png and out/trace.json");
    }
}
