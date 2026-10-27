//! Repeatable timings for the hot paths: a settled pile of 3000 balls, then the median of many
//! physics steps and many rendered frames (shadow + main pass) of the same snapshot.
//!
//!   cargo run --release --example bench -- [--threads N]

use std::time::Instant;

use forja::{
    frame::Renderer,
    math::{Rng, Vec3, vec3},
    physics::PhysicsWorld,
    prelude::*,
    render::{Instance, Snapshot},
    scene::Sun,
    simd,
};

fn median_ms(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2] * 1000.0
}

fn main() {
    let threads = std::env::args().skip_while(|a| a != "--threads").nth(1).and_then(|v| v.parse().ok());
    let jobs = match threads {
        Some(n) => JobSystem::new(usize::max(n, 1) - 1),
        None => JobSystem::with_available_parallelism(),
    };
    println!("{} threads, simd: {}", jobs.thread_count(), simd::IMPLEMENTATION);

    let mut phys = PhysicsWorld::default();
    let mut rng = Rng::new(0x5eed);
    for _ in 0..3000 {
        let r = rng.range(0.18, 0.32);
        phys.add_body(vec3(rng.range(-9.0, 9.0), rng.range(1.0, 25.0), rng.range(-9.0, 9.0)), r, r * r * r);
    }
    for _ in 0..600 {
        phys.step(&jobs, 1.0 / 60.0);
    }
    forja::profile::set_enabled(true);
    let _ = forja::profile::take_events();
    let physics: Vec<f64> = (0..200)
        .map(|_| {
            let t = Instant::now();
            phys.step(&jobs, 1.0 / 60.0);
            t.elapsed().as_secs_f64()
        })
        .collect();
    forja::profile::set_enabled(false);
    let events = forja::profile::take_events();
    println!("physics breakdown (self time, all threads, per step):");
    forja::profile::Report::summarize(&events, jobs.thread_count()).print(200);

    let mut renderer = SoftwareRenderer::new(960, 540);
    let sphere = renderer.add_mesh_lods(vec![(Mesh::sphere(20, 12), 24.0), (Mesh::sphere(12, 8), 8.0), (Mesh::sphere(8, 5), 0.0)]);
    let cube = renderer.add_mesh(Mesh::cube());
    let mut snap = Snapshot {
        camera: Camera { eye: vec3(16.0, 16.0, 22.0), target: vec3(0.0, 2.0, 0.0), fov_y: 0.9, ..Camera::default() },
        sun: Sun { shadow_center: vec3(0.0, 2.0, 0.0), shadow_radius: 16.0, ..Sun::default() },
        ..Snapshot::default()
    };
    snap.instances.push(Instance { mesh: cube, pos: vec3(0.0, -0.5, 0.0), scale: vec3(10.0, 0.5, 10.0), color: Vec3::splat(0.4), ..Instance::default() });
    for (p, r) in phys.pos.iter().zip(&phys.radius) {
        snap.instances.push(Instance { mesh: sphere, pos: *p, scale: Vec3::splat(*r), color: vec3(0.8, 0.3, 0.2), ..Instance::default() });
    }
    for _ in 0..10 {
        renderer.render(&snap, &jobs);
    }
    let render: Vec<f64> = (0..100)
        .map(|_| {
            let t = Instant::now();
            renderer.render(&snap, &jobs);
            t.elapsed().as_secs_f64()
        })
        .collect();

    println!("physics step (3000 bodies, 4 substeps x 2 iterations): {:6.3} ms median", median_ms(physics));
    println!("render frame (960x540 + 1024^2 shadow, {} tris):   {:6.3} ms median", renderer.stats.triangles, median_ms(render));
    let _ = forja::image::write_png(std::path::Path::new("out/bench.png"), renderer.width, renderer.height, &renderer.color);
}
