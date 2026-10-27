//! The frame model.
//!
//! A frame is a fixed sequence of phases, each with a clear owner of the data:
//!
//! ```text
//!            main thread owns &mut World            |  render job owns (Renderer, Snapshot)
//!  frame N:  [begin] [tick]*k [extract N] ──hand-off──▶ [render N] ........................
//!  frame N+1:                        [begin] [tick]*k [extract N+1] ─wait N─▶ [render N+1] ...
//! ```
//!
//! - **begin** (variable dt): input, camera, anything tied to the display rate.
//! - **tick** (fixed dt, 0..max_ticks per frame): simulation. Fixed step = stable physics and
//!   deterministic replays. The clock accumulates real time and tells you how many ticks to run.
//! - **extract**: the only sync point between simulation and rendering. Copies what the renderer
//!   needs out of the world into a `Snapshot` (plain data, owned).
//! - **render**: runs as a background job *while the next frame simulates*. It owns the renderer
//!   and the snapshot by value, so there is no shared mutable state and no locks; ownership comes
//!   back to the main thread when the next frame's extract is done.
//!
//! All phases use the same worker pool, so render batches fill the gaps of the simulation's
//! dependency graph and vice versa — the CPU stays busy instead of waiting at a barrier.

use std::time::Instant;

use crate::{
    jobs::{JobHandle, Jobs},
    profile,
    world::World,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInfo {
    /// Frame counter.
    pub frame: u64,
    /// In `begin`: real seconds since the previous frame. In a tick: the fixed step.
    pub dt: f32,
    /// Seconds of simulated time at the start of this frame.
    pub time: f64,
    pub fixed_dt: f32,
    /// Index of the first tick of this frame (or of this tick, inside `ticks()`).
    pub tick: u64,
    /// Number of fixed ticks this frame runs.
    pub ticks: u32,
    /// How far between the last tick and the next one the frame is, 0..1. For interpolation.
    pub alpha: f32,
}

impl FrameInfo {
    /// The tick infos to feed the fixed-step schedule this frame.
    pub fn tick_infos(&self) -> impl Iterator<Item = FrameInfo> + use<> {
        let base = *self;
        (0..base.ticks).map(move |i| FrameInfo { tick: base.tick + i as u64, dt: base.fixed_dt, ..base })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockMode {
    /// Accumulates wall-clock time; runs as many ticks as fit (capped).
    RealTime,
    /// Exactly one tick per frame, dt = fixed step. Same inputs => same results. For tests, replays,
    /// servers and offline runs.
    Deterministic,
}

pub struct FrameClock {
    fixed_dt: f64,
    max_ticks: u32,
    mode: ClockMode,
    accumulator: f64,
    last: Option<Instant>,
    frame: u64,
    tick: u64,
    time: f64,
}

impl FrameClock {
    pub fn new(tick_hz: f64) -> FrameClock {
        FrameClock {
            fixed_dt: 1.0 / tick_hz,
            max_ticks: 4,
            mode: ClockMode::RealTime,
            accumulator: 0.0,
            last: None,
            frame: 0,
            tick: 0,
            time: 0.0,
        }
    }

    pub fn deterministic(tick_hz: f64) -> FrameClock {
        FrameClock { mode: ClockMode::Deterministic, ..FrameClock::new(tick_hz) }
    }

    /// Cap on ticks per frame. When the machine can't keep up, simulation slows down instead of
    /// spiralling (each late frame scheduling even more ticks).
    pub fn max_ticks(mut self, n: u32) -> FrameClock {
        self.max_ticks = n.max(1);
        self
    }

    pub fn begin(&mut self) -> FrameInfo {
        let now = Instant::now();
        let real_dt = self.last.map(|l| (now - l).as_secs_f64()).unwrap_or(self.fixed_dt);
        self.last = Some(now);
        let (dt, ticks) = match self.mode {
            ClockMode::Deterministic => (self.fixed_dt, 1),
            ClockMode::RealTime => {
                let dt = real_dt.min(0.25);
                self.accumulator += dt;
                let mut ticks = (self.accumulator / self.fixed_dt) as u32;
                if ticks > self.max_ticks {
                    ticks = self.max_ticks;
                    self.accumulator = self.fixed_dt * ticks as f64;
                }
                self.accumulator -= self.fixed_dt * ticks as f64;
                (dt, ticks)
            }
        };
        let info = FrameInfo {
            frame: self.frame,
            dt: dt as f32,
            time: self.time,
            fixed_dt: self.fixed_dt as f32,
            tick: self.tick,
            ticks,
            alpha: (self.accumulator / self.fixed_dt) as f32,
        };
        self.frame += 1;
        self.tick += ticks as u64;
        self.time += self.fixed_dt * ticks as f64;
        info
    }
}

/// The render side of the frame. `extract` runs on the main thread with read access to the world;
/// `render` runs as a background job with exclusive access to the renderer and the snapshot.
pub trait Renderer: Send + 'static {
    type Snapshot: Default + Send + 'static;
    fn extract(world: &World, jobs: &Jobs, frame: &FrameInfo, out: &mut Self::Snapshot);
    fn render(&mut self, snapshot: &Self::Snapshot, jobs: &Jobs);
}

/// Keeps one frame in flight: frame N renders while frame N+1 simulates. Snapshots are recycled
/// (two of them ping-pong), so steady state allocates nothing.
pub struct FramePipeline<R: Renderer> {
    idle: Option<R>,
    spare: Vec<R::Snapshot>,
    in_flight: Option<JobHandle<(R, R::Snapshot)>>,
    /// Frame number of the render in flight.
    in_flight_frame: u64,
}

impl<R: Renderer> FramePipeline<R> {
    pub fn new(renderer: R) -> FramePipeline<R> {
        FramePipeline { idle: Some(renderer), spare: Vec::new(), in_flight: None, in_flight_frame: 0 }
    }

    /// Extract this frame, wait for the previous one, give it to `present`, start this one.
    /// `present` receives the renderer holding the *previous* frame's output.
    pub fn submit(&mut self, world: &World, jobs: &Jobs, frame: &FrameInfo, present: impl FnOnce(&mut R)) {
        let _frame = profile::frame_tag(frame.frame);
        let mut snapshot = self.spare.pop().unwrap_or_default();
        {
            let _p = profile::scope_kind(profile::Kind::Phase, "extract");
            R::extract(world, jobs, frame, &mut snapshot);
        }
        let had_output = self.in_flight.is_some();
        let rendered = self.in_flight_frame;
        let mut renderer = self.retire();
        if had_output {
            // Shows the previous frame's image, so it counts for that frame.
            let _frame = profile::frame_tag(rendered);
            let _p = profile::scope_kind(profile::Kind::Phase, "present");
            present(&mut renderer);
        }
        let job_jobs = jobs.clone();
        self.in_flight_frame = frame.frame;
        self.in_flight = Some(jobs.spawn_named("render", move || {
            renderer.render(&snapshot, &job_jobs);
            (renderer, snapshot)
        }));
    }

    /// Waits for the frame in flight and returns the renderer with its output (e.g. for a
    /// screenshot or before shutdown).
    pub fn finish(&mut self) -> &mut R {
        let r = self.retire();
        self.idle.insert(r)
    }

    fn retire(&mut self) -> R {
        match self.in_flight.take() {
            Some(handle) => {
                let _p = profile::scope_kind(profile::Kind::Phase, "wait render");
                let (renderer, snapshot) = handle.join();
                self.spare.push(snapshot);
                renderer
            }
            None => self.idle.take().expect("renderer missing"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobSystem;

    #[test]
    fn deterministic_clock_runs_one_tick_per_frame() {
        let mut c = FrameClock::deterministic(50.0);
        for i in 0..10 {
            let f = c.begin();
            assert_eq!((f.frame, f.tick, f.ticks), (i, i, 1));
            assert!((f.dt - 0.02).abs() < 1e-6);
        }
    }

    #[derive(Default)]
    struct Counter {
        rendered: Vec<u64>,
    }
    impl Renderer for Counter {
        type Snapshot = u64;
        fn extract(world: &World, _: &Jobs, _: &FrameInfo, out: &mut u64) {
            *out = *world.res::<u64>();
        }
        fn render(&mut self, s: &u64, _: &Jobs) {
            self.rendered.push(*s);
        }
    }

    #[test]
    fn pipeline_presents_previous_frame_and_keeps_order() {
        let jobs = JobSystem::new(2);
        let mut world = World::new();
        world.insert_resource(0u64);
        let mut pipe = FramePipeline::new(Counter::default());
        let mut presented = Vec::new();
        for i in 0..5u64 {
            *world.resource_mut::<u64>() = i * 10;
            pipe.submit(&world, &jobs, &FrameInfo::default(), |r| presented.push(*r.rendered.last().unwrap()));
        }
        assert_eq!(presented, [0, 10, 20, 30]);
        assert_eq!(pipe.finish().rendered, [0, 10, 20, 30, 40]);
    }
}
