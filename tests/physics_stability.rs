//! Same setup as the `rain` example: balls rain into a 20x20 box and must come to rest.

use forja::{
    jobs::JobSystem,
    math::{Rng, vec3},
    physics::PhysicsWorld,
};

#[test]
fn rain_settles_without_launching_balls() {
    let jobs = JobSystem::with_available_parallelism();
    let mut w = PhysicsWorld::default();
    w.bounds_min = vec3(-10.0, 0.0, -10.0);
    w.bounds_max = vec3(10.0, f32::INFINITY, 10.0);
    let mut rng = Rng::new(0x5eed);
    let mut worst_late_speed = 0.0f32;
    for tick in 0..900 {
        if w.len() < 3000 {
            for _ in 0..25 {
                let r = rng.range(0.18, 0.32);
                let p = vec3(rng.range(-9.0, 9.0), rng.range(12.0, 18.0), rng.range(-9.0, 9.0));
                let id = w.add_body(p, r, r * r * r);
                w.vel[id.0 as usize] = vec3(rng.range(-1.0, 1.0), rng.range(-2.0, 0.0), rng.range(-1.0, 1.0));
            }
        }
        w.step(&jobs, 1.0 / 60.0);
        if tick > 600 {
            let (i, v) = w.vel.iter().enumerate().max_by(|a, b| a.1.length().total_cmp(&b.1.length())).unwrap();
            if v.length() > worst_late_speed {
                worst_late_speed = v.length();
                eprintln!("tick {tick}: body {i} at {:.2} m/s, {:?}", v.length(), w.pos[i]);
            }
        }
    }
    let top = w.pos.iter().zip(&w.radius).map(|(p, r)| p.y - r).fold(0.0f32, f32::max);
    assert!(top < 1.5, "a ball is still {top:.2} above the floor");
    // Settling balls may still roll off the pile (~1-2 m/s); a launched ball goes 10x faster.
    assert!(worst_late_speed < 4.0, "a ball is moving at {worst_late_speed:.2} m/s after settling");
}
