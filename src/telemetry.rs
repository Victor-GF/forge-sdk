//! Frame telemetry on top of `profile` (scopes, flows) and `alloc` (heap counters), read from the
//! top down:
//!
//! - **macro, the frame pipeline**: when each frame simulates, renders and reaches the screen,
//!   its latency, and how many threads work, wait or sleep while it happens;
//! - **meso, the systems**: when each system of the tick starts and ends, and on which thread;
//! - **micro, passes and jobs**: how each pass scales with threads, its footprint against the
//!   caches, and how long jobs wait in the queue before a thread takes them.
//!
//! The timeline goes to Perfetto (`profile::write_trace`) with extra tracks built here: one track
//! per pipeline stage and per-frame counters. The tables go to the terminal (`print_report`).

use std::sync::Mutex;

use crate::{
    alloc::{self, AllocStats},
    cpu::Cache,
    jobs::Jobs,
    profile::{self, Counter, Event, Flow, FrameSpan, Kind, Span},
};

/// Records one `FrameSpan` per frame plus memory counters.
#[derive(Default)]
pub struct Recorder {
    pub frames: Vec<FrameSpan>,
    start_ns: u64,
    alloc_start: AllocStats,
    jobs_start: u64,
}

impl Recorder {
    pub fn begin_frame(&mut self, jobs: &Jobs) {
        self.start_ns = profile::now_ns();
        self.alloc_start = alloc::stats();
        self.jobs_start = jobs.jobs_run().iter().sum();
    }

    /// Closes frame number `frame` (the clock's, as in `FrameInfo::frame`). Call before anything
    /// else allocates, so the frame's counts stay the engine's own.
    pub fn end_frame(&mut self, jobs: &Jobs, frame: u64, ticks: u32) {
        let a = alloc::stats().since(&self.alloc_start);
        self.frames.push(FrameSpan {
            index: frame,
            start_ns: self.start_ns,
            end_ns: profile::now_ns(),
            ticks,
            allocs: a.allocs,
            alloc_bytes: a.bytes_allocated,
            live_bytes: a.live_bytes,
            jobs: jobs.jobs_run().iter().sum::<u64>() - self.jobs_start,
        });
        profile::counter("heap live (MiB)", a.live_bytes as f64 / (1024.0 * 1024.0));
        profile::counter("allocations per frame", a.allocs as f64);
    }

    /// Samples heap bytes per subsystem into one counter track each.
    pub fn sample_memory(&mut self, memory: &[(&'static str, usize)]) {
        for &(name, bytes) in memory {
            profile::counter(memory_track(name), bytes as f64 / 1024.0);
        }
    }
}

/// "mem: <name> (KiB)", interned so counters keep `&'static str` names.
fn memory_track(name: &'static str) -> &'static str {
    static NAMES: Mutex<Vec<(&'static str, &'static str)>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap();
    if let Some(&(_, track)) = names.iter().find(|(n, _)| *n == name) {
        return track;
    }
    let track: &'static str = Box::leak(format!("mem: {name} (KiB)").into_boxed_str());
    names.push((name, track));
    track
}

/// Everything recorded for one run.
pub struct Capture {
    pub threads: usize,
    pub events: Vec<Event>,
    pub counters: Vec<Counter>,
    pub flows: Vec<Flow>,
    pub frames: Vec<FrameSpan>,
    /// Jobs each thread ran during the recording.
    pub jobs_per_thread: Vec<u64>,
    /// Heap bytes per subsystem at the end.
    pub memory: Vec<(&'static str, usize)>,
}

// --- macro: the frame pipeline ------------------------------------------------------------------

/// The life of one frame: simulated, extracted, rendered (as a background job, during the next
/// frame), presented (at the next frame's submit).
#[derive(Clone, Copy, Debug)]
pub struct FrameLife {
    pub frame: u32,
    pub sim: (u64, u64),
    pub extract: Option<(u64, u64)>,
    pub render: Option<(u64, u64)>,
    pub present: Option<(u64, u64)>,
}

impl FrameLife {
    /// From the start of the simulation to the image being presented.
    pub fn latency_ns(&self) -> Option<u64> {
        self.present.map(|p| p.1 - self.sim.0)
    }
}

pub fn frame_lives(events: &[Event]) -> Vec<FrameLife> {
    let mut lives: Vec<FrameLife> = Vec::new();
    let span = |e: &Event| (e.start_ns, e.start_ns + e.dur_ns);
    let grow = |a: Option<(u64, u64)>, b: (u64, u64)| Some(a.map_or(b, |a| (a.0.min(b.0), a.1.max(b.1))));
    for e in events.iter().filter(|e| e.frame != profile::NO_FRAME) {
        let i = match lives.iter().position(|l| l.frame == e.frame) {
            Some(i) => i,
            None => {
                lives.push(FrameLife { frame: e.frame, sim: (u64::MAX, 0), extract: None, render: None, present: None });
                lives.len() - 1
            }
        };
        let l = &mut lives[i];
        match (e.kind, e.name) {
            (Kind::Schedule, _) => l.sim = (l.sim.0.min(e.start_ns), l.sim.1.max(e.start_ns + e.dur_ns)),
            (Kind::Phase, "extract") => l.extract = grow(l.extract, span(e)),
            (Kind::Phase, "present") => l.present = grow(l.present, span(e)),
            (Kind::Job, "render") => l.render = grow(l.render, span(e)),
            _ => {}
        }
    }
    lives.retain(|l| l.sim.1 > 0);
    lives.sort_by_key(|l| l.frame);
    lives
}

/// Pipeline tracks for Perfetto: one per stage, plus latency on two alternating tracks (it spans
/// more than a frame, so consecutive frames overlap).
pub fn pipeline_spans(lives: &[FrameLife]) -> Vec<Span> {
    let mut out = Vec::new();
    for l in lives {
        let ms = |s: (u64, u64)| (s.1 - s.0) as f64 / 1e6;
        let mut push = |track: &'static str, s: (u64, u64), args: Vec<(&'static str, f64)>| {
            out.push(Span { track, name: format!("frame {}", l.frame), start_ns: s.0, end_ns: s.1, args });
        };
        push("1 pipeline: simulate", l.sim, vec![("ms", ms(l.sim))]);
        if let Some(r) = l.render {
            push("2 pipeline: render", r, vec![("ms", ms(r))]);
        }
        if let Some(p) = l.present {
            push("3 pipeline: present", p, vec![("ms", ms(p))]);
            let lat = (l.sim.0, p.1);
            let track = if l.frame % 2 == 0 { "4 latency (even frames)" } else { "4 latency (odd frames)" };
            push(track, lat, vec![("latency_ms", ms(lat))]);
        }
    }
    out
}

/// Per-frame counters: how many threads (on average over the frame) work, wait and sleep, the
/// mean time jobs sat in the queue, and each frame's latency.
pub fn frame_counters(events: &[Event], frames: &[FrameSpan], lives: &[FrameLife]) -> Vec<Counter> {
    let self_ns = profile::self_times(events);
    let mut out = Vec::new();
    for f in frames {
        let (mut work, mut wait, mut idle, mut queued, mut jobs) = (0.0, 0.0, 0.0, 0u64, 0u64);
        for (e, &s) in events.iter().zip(&self_ns) {
            let (a, b) = (e.start_ns.max(f.start_ns), (e.start_ns + e.dur_ns).min(f.end_ns));
            if b <= a || e.dur_ns == 0 {
                continue;
            }
            // Self time spread evenly over the event, clipped to the frame.
            let part = s as f64 * (b - a) as f64 / e.dur_ns as f64;
            match e.kind {
                Kind::Wait => wait += part,
                Kind::Idle => idle += part,
                _ => work += part,
            }
            if e.kind == Kind::Job && e.start_ns >= f.start_ns && e.start_ns < f.end_ns {
                queued += e.queued_ns as u64;
                jobs += 1;
            }
        }
        let len = (f.end_ns - f.start_ns).max(1) as f64;
        let ts = f.end_ns;
        out.push(Counter { name: "threads working", ts_ns: ts, value: work / len });
        out.push(Counter { name: "threads waiting", ts_ns: ts, value: wait / len });
        out.push(Counter { name: "threads idle", ts_ns: ts, value: idle / len });
        out.push(Counter { name: "frame time (ms)", ts_ns: ts, value: len / 1e6 });
        if jobs > 0 {
            out.push(Counter { name: "job queue wait (us)", ts_ns: ts, value: queued as f64 / jobs as f64 / 1000.0 });
        }
    }
    for l in lives {
        if let (Some(lat), Some(p)) = (l.latency_ns(), l.present) {
            out.push(Counter { name: "frame latency (ms)", ts_ns: p.1, value: lat as f64 / 1e6 });
        }
    }
    out.sort_by_key(|c| c.ts_ns);
    out
}

// --- summary ------------------------------------------------------------------------------------

/// Mean and 95th percentile.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stat {
    pub mean: f64,
    pub p95: f64,
    pub max: f64,
}

fn stat(mut v: Vec<f64>) -> Stat {
    if v.is_empty() {
        return Stat::default();
    }
    v.sort_by(f64::total_cmp);
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    Stat { mean, p95: v[((v.len() - 1) as f64 * 0.95).round() as usize], max: *v.last().unwrap() }
}

#[derive(Clone, Debug, Default)]
pub struct Macro {
    pub frame_ms: Stat,
    pub latency_ms: Stat,
    pub sim_ms: Stat,
    pub render_ms: Stat,
    /// Time render N and the simulation of N+1 run at the same time.
    pub overlap_ms: Stat,
    /// Thread-equivalents working, waiting and asleep, averaged over the recording.
    pub working: f64,
    pub waiting: f64,
    pub idle: f64,
}

/// One system of the fixed-step schedule, per frame.
#[derive(Clone, Debug)]
pub struct SystemTiming {
    pub name: &'static str,
    /// Start, relative to the start of its frame's simulation.
    pub start_ms: f64,
    pub dur_ms: f64,
    /// Thread that ran it most often.
    pub thread: u16,
    /// Fraction of frames it ran on that thread.
    pub thread_share: f64,
}

/// Jobs grouped by name (the scope that spawned them).
#[derive(Clone, Debug)]
pub struct JobGroup {
    pub name: &'static str,
    pub per_frame: f64,
    pub dur_us: f64,
    pub queued_us: Stat,
    /// Fraction run by the thread that spawned them while it waited (thread 0 for the main thread
    /// is counted as "main").
    pub on_main: f64,
}

/// One scope name over a recorded run, per frame.
#[derive(Clone, Debug)]
pub struct Pass {
    pub name: &'static str,
    /// Time during which at least one thread was inside this pass (union of its intervals).
    pub wall_ms: f64,
    /// Self time summed over all threads: the work, wherever it ran.
    pub cpu_ms: f64,
    pub calls: f64,
    pub allocs: f64,
    /// Largest declared footprint of one call (0 if the scope doesn't declare bytes).
    pub bytes: u64,
    /// Declared bytes divided by the duration of the calls that declared them.
    pub gb_per_s: f64,
    /// Other scopes open inside this one. Then `cpu_ms` (self time) leaves the children out
    /// while `wall_ms` includes them, so their ratio means nothing.
    pub has_children: bool,
}

impl Pass {
    /// Average number of threads busy in this pass while it runs (leaf passes only).
    pub fn parallelism(&self) -> Option<f64> {
        (!self.has_children && self.wall_ms > 0.0).then(|| self.cpu_ms / self.wall_ms)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ThreadTime {
    /// Fractions of the recorded wall time.
    pub work: f64,
    /// Waiting on a scope or a job while helping with other jobs.
    pub wait: f64,
    /// Asleep: nothing queued.
    pub idle: f64,
    pub jobs_per_frame: f64,
}

#[derive(Clone, Debug)]
pub struct RunSummary {
    pub threads: usize,
    pub frames: usize,
    pub macro_: Macro,
    pub systems: Vec<SystemTiming>,
    pub jobs: Vec<JobGroup>,
    pub passes: Vec<Pass>,
    pub thread_time: Vec<ThreadTime>,
    pub allocs_per_frame: f64,
    pub alloc_kib_per_frame: f64,
    pub heap_live_bytes: u64,
    pub memory: Vec<(&'static str, usize)>,
    /// Largest value of each counter track that isn't a memory section.
    pub counter_peaks: Vec<(&'static str, f64)>,
}

impl RunSummary {
    pub fn pass(&self, name: &str) -> Option<&Pass> {
        self.passes.iter().find(|p| p.name == name)
    }
}

pub fn summarize(c: &Capture, lives: &[FrameLife]) -> RunSummary {
    let events = &c.events;
    let n = c.frames.len().max(1) as f64;
    let wall_ns = c.frames.last().map_or(0, |f| f.end_ns) - c.frames.first().map_or(0, |f| f.start_ns);
    let self_ns = profile::self_times(events);
    let ms = |s: (u64, u64)| (s.1 - s.0) as f64 / 1e6;

    // Macro. Only frames whose whole life was recorded.
    let recorded: Vec<&FrameLife> = lives.iter().filter(|l| l.render.is_some() && l.present.is_some()).collect();
    let overlap = lives.windows(2).filter_map(|w| {
        let r = w[0].render?;
        let s = w[1].sim;
        Some((r.1.min(s.1).saturating_sub(r.0.max(s.0))) as f64 / 1e6)
    });
    let mut macro_ = Macro {
        frame_ms: stat(c.frames.iter().map(|f| (f.end_ns - f.start_ns) as f64 / 1e6).collect()),
        latency_ms: stat(recorded.iter().filter_map(|l| l.latency_ns()).map(|x| x as f64 / 1e6).collect()),
        sim_ms: stat(lives.iter().map(|l| ms(l.sim)).collect()),
        render_ms: stat(recorded.iter().filter_map(|l| l.render).map(ms).collect()),
        overlap_ms: stat(overlap.collect()),
        ..Macro::default()
    };

    // Threads.
    let mut thread_time = vec![ThreadTime::default(); c.threads];
    for (e, &s) in events.iter().zip(&self_ns) {
        let Some(t) = thread_time.get_mut(e.thread as usize) else { continue };
        let f = s as f64 / wall_ns.max(1) as f64;
        match e.kind {
            Kind::Wait => t.wait += f,
            Kind::Idle => t.idle += f,
            _ => t.work += f,
        }
    }
    for (t, &jobs) in thread_time.iter_mut().zip(&c.jobs_per_thread) {
        t.jobs_per_frame = jobs as f64 / n;
    }
    macro_.working = thread_time.iter().map(|t| t.work).sum();
    macro_.waiting = thread_time.iter().map(|t| t.wait).sum();
    macro_.idle = thread_time.iter().map(|t| t.idle).sum();

    // Meso: systems, relative to their frame's simulation start.
    let mut systems: Vec<SystemTiming> = Vec::new();
    let mut names: Vec<&'static str> = events.iter().filter(|e| e.kind == Kind::System).map(|e| e.name).collect();
    names.dedup();
    let mut seen = Vec::new();
    for name in names {
        if seen.contains(&name) {
            continue;
        }
        seen.push(name);
        let runs: Vec<&Event> = events.iter().filter(|e| e.kind == Kind::System && e.name == name).collect();
        let mut starts = Vec::new();
        let mut threads = vec![0usize; c.threads.max(1)];
        for e in &runs {
            if let Some(l) = lives.iter().find(|l| l.frame == e.frame) {
                starts.push((e.start_ns.saturating_sub(l.sim.0)) as f64 / 1e6);
            }
            if let Some(t) = threads.get_mut(e.thread as usize) {
                *t += 1;
            }
        }
        let (thread, count) = threads.iter().enumerate().max_by_key(|(_, c)| **c).map_or((0, 0), |(t, c)| (t, *c));
        systems.push(SystemTiming {
            name,
            start_ms: stat(starts).mean,
            dur_ms: stat(runs.iter().map(|e| e.dur_ns as f64 / 1e6).collect()).mean,
            thread: thread as u16,
            thread_share: count as f64 / runs.len().max(1) as f64,
        });
    }
    systems.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));

    // Micro: jobs by name.
    let mut jobs: Vec<JobGroup> = Vec::new();
    let mut job_names: Vec<&'static str> = events.iter().filter(|e| e.kind == Kind::Job).map(|e| e.name).collect();
    job_names.sort_unstable();
    job_names.dedup();
    for name in job_names {
        let js: Vec<&Event> = events.iter().filter(|e| e.kind == Kind::Job && e.name == name).collect();
        jobs.push(JobGroup {
            name,
            per_frame: js.len() as f64 / n,
            dur_us: js.iter().map(|e| e.dur_ns as f64).sum::<f64>() / js.len() as f64 / 1000.0,
            queued_us: stat(js.iter().map(|e| e.queued_ns as f64 / 1000.0).collect()),
            on_main: js.iter().filter(|e| e.thread == 0).count() as f64 / js.len() as f64,
        });
    }
    jobs.sort_by(|a, b| (b.per_frame * b.dur_us).total_cmp(&(a.per_frame * a.dur_us)));

    // Micro: passes by name (pass slices plus the jobs named after them).
    let parents = parent_names(events);
    let mut names: Vec<&'static str> = events.iter().map(|e| e.name).collect();
    names.sort_unstable();
    names.dedup();
    let mut passes = Vec::new();
    for name in names {
        let mut spans: Vec<(u64, u64)> = Vec::new();
        let (mut cpu, mut calls, mut allocs, mut bytes, mut declared, mut declared_ns) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
        for (e, &s) in events.iter().zip(&self_ns).filter(|(e, _)| e.name == name) {
            spans.push((e.start_ns, e.start_ns + e.dur_ns));
            cpu += s;
            calls += 1;
            allocs += e.allocs as u64;
            if e.bytes > 0 {
                bytes = bytes.max(e.bytes);
                declared += e.bytes;
                declared_ns += e.dur_ns;
            }
        }
        passes.push(Pass {
            name,
            wall_ms: union_ns(&mut spans) as f64 / 1e6 / n,
            cpu_ms: cpu as f64 / 1e6 / n,
            calls: calls as f64 / n,
            allocs: allocs as f64 / n,
            bytes,
            gb_per_s: if declared_ns > 0 { declared as f64 / declared_ns as f64 } else { 0.0 },
            has_children: parents.contains(&name),
        });
    }
    passes.sort_by(|a, b| b.cpu_ms.total_cmp(&a.cpu_ms));

    let mut peaks: Vec<(&'static str, f64)> = Vec::new();
    for c in c.counters.iter().filter(|c| !c.name.starts_with("mem: ")) {
        match peaks.iter_mut().find(|(n, _)| *n == c.name) {
            Some(p) => p.1 = p.1.max(c.value),
            None => peaks.push((c.name, c.value)),
        }
    }

    RunSummary {
        threads: c.threads,
        frames: c.frames.len(),
        macro_,
        systems,
        jobs,
        passes,
        thread_time,
        allocs_per_frame: c.frames.iter().map(|f| f.allocs).sum::<u64>() as f64 / n,
        alloc_kib_per_frame: c.frames.iter().map(|f| f.alloc_bytes).sum::<u64>() as f64 / 1024.0 / n,
        heap_live_bytes: c.frames.last().map_or(0, |f| f.live_bytes),
        memory: c.memory.clone(),
        counter_peaks: peaks,
    }
}

/// Names of scopes that have another work scope opened inside them on the same thread.
fn parent_names(events: &[Event]) -> Vec<&'static str> {
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&i| (events[i].thread, events[i].start_ns, std::cmp::Reverse(events[i].dur_ns)));
    let (mut stack, mut thread, mut out): (Vec<usize>, u16, Vec<&'static str>) = (Vec::new(), u16::MAX, Vec::new());
    for &i in &order {
        let e = &events[i];
        if e.thread != thread {
            stack.clear();
            thread = e.thread;
        }
        while stack.last().is_some_and(|&top| events[top].start_ns + events[top].dur_ns <= e.start_ns) {
            stack.pop();
        }
        if let Some(&parent) = stack.last()
            && e.kind.is_work()
            && !out.contains(&events[parent].name)
        {
            out.push(events[parent].name);
        }
        stack.push(i);
    }
    out
}

/// Total length covered by the intervals.
fn union_ns(spans: &mut [(u64, u64)]) -> u64 {
    spans.sort_unstable();
    let (mut total, mut cur) = (0, None::<(u64, u64)>);
    for &(s, e) in spans.iter() {
        cur = match cur {
            Some((cs, ce)) if s <= ce => Some((cs, ce.max(e))),
            Some((cs, ce)) => {
                total += ce - cs;
                Some((s, e))
            }
            None => Some((s, e)),
        };
    }
    total + cur.map_or(0, |(s, e)| e - s)
}

/// Smallest data cache level that holds `bytes` ("L1", "L2", "L3"), else "RAM".
pub fn fits_in(bytes: u64, caches: &[Cache]) -> String {
    let mut levels: Vec<&Cache> = caches.iter().filter(|c| c.kind != "instruction").collect();
    levels.sort_by_key(|c| c.level);
    levels.iter().find(|c| bytes <= c.bytes as u64).map_or("RAM".into(), |c| format!("L{}", c.level))
}

fn kib(b: u64) -> String {
    match b {
        0 => "-".into(),
        b if b < 1 << 20 => format!("{:.0} KiB", b as f64 / 1024.0),
        b => format!("{:.1} MiB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Prints the report from the top down: the machine, the frame pipeline per thread count, the
/// systems of the tick, passes and jobs, threads and memory of the last run.
pub fn print_report(runs: &[RunSummary], caches: &[Cache]) {
    let Some(last) = runs.last() else { return };
    let base = &runs[0];

    let levels: Vec<String> =
        caches.iter().filter(|c| c.kind != "instruction").map(|c| format!("L{} {} (x{} threads)", c.level, kib(c.bytes as u64), c.shared_by)).collect();
    println!("\n== machine ==\n  caches: {}", levels.join(", "));

    println!("\n== macro: the frame pipeline (ms; mean / p95) ==");
    println!("  threads      frame      latency   simulate     render    overlap   speedup   working  waiting  idle");
    for r in runs {
        let m = &r.macro_;
        let p = |s: Stat| format!("{:>5.2}/{:<5.2}", s.mean, s.p95);
        println!(
            "  {:>7}  {}  {}  {}  {}  {:>8.2}  {:>7.2}x  {:>8.1}  {:>7.1}  {:>4.1}",
            r.threads,
            p(m.frame_ms),
            p(m.latency_ms),
            p(m.sim_ms),
            p(m.render_ms),
            m.overlap_ms.mean,
            base.macro_.frame_ms.mean / m.frame_ms.mean,
            m.working,
            m.waiting,
            m.idle
        );
    }
    println!("  latency: start of simulation to present. overlap: render N running during simulate N+1.");
    println!("  working/waiting/idle: thread-equivalents averaged over the recording.");

    println!("\n== meso: systems of the frame ({} threads; start relative to the frame's simulation) ==", last.threads);
    println!("  {:<18} {:>9} {:>9}   thread", "system", "start", "duration");
    for s in &last.systems {
        let bar_start = (s.start_ms * 4.0).round() as usize;
        let bar_len = ((s.dur_ms * 4.0).round() as usize).max(1);
        println!(
            "  {:<18} {:>7.2}ms {:>7.2}ms   {:<9} {:>3.0}%  {}{}",
            s.name,
            s.start_ms,
            s.dur_ms,
            profile::thread_name(s.thread as usize),
            s.thread_share * 100.0,
            " ".repeat(bar_start.min(60)),
            "#".repeat(bar_len.min(60))
        );
    }

    println!("\n== micro: passes, wall ms/frame by thread count ==");
    print!("  {:<22}", "pass");
    for r in runs {
        print!("{:>8}", format!("{}T", r.threads));
    }
    println!("  speedup  threads busy  footprint  fits  GB/s");
    let mut rows: Vec<&Pass> = last.passes.iter().filter(|p| p.name != profile::WAIT && p.name != profile::IDLE && p.wall_ms > 0.02).collect();
    rows.sort_by(|a, b| b.wall_ms.total_cmp(&a.wall_ms));
    for p in rows.iter().take(22) {
        print!("  {:<22}", p.name);
        for r in runs {
            match r.pass(p.name) {
                Some(q) => print!("{:>8.2}", q.wall_ms),
                None => print!("{:>8}", "-"),
            }
        }
        let speedup = base.pass(p.name).map_or(0.0, |b| b.wall_ms / p.wall_ms.max(1e-9));
        let gbps = if p.gb_per_s > 0.0 { format!("{:.1}", p.gb_per_s) } else { "-".into() };
        let fits = if p.bytes > 0 { fits_in(p.bytes, caches) } else { "-".into() };
        let busy = p.parallelism().map_or("-".into(), |x| format!("{x:.1}"));
        println!("  {:>6.2}x  {:>12}  {:>9}  {:>4}  {:>4}", speedup, busy, kib(p.bytes), fits, gbps);
    }

    println!("\n== micro: jobs ({} threads) ==", last.threads);
    println!("  {:<22} {:>9} {:>9}  {:>17}  {:>8}", "spawned by", "per frame", "avg us", "queue us mean/p95", "on main");
    for j in last.jobs.iter().take(14) {
        println!(
            "  {:<22} {:>9.1} {:>9.1}  {:>8.1} /{:>7.1}  {:>7.0}%",
            j.name,
            j.per_frame,
            j.dur_us,
            j.queued_us.mean,
            j.queued_us.p95,
            j.on_main * 100.0
        );
    }

    println!("\n== threads ({} threads) ==", last.threads);
    println!("  thread     work   wait(help)   idle   other   jobs/frame");
    for (i, t) in last.thread_time.iter().enumerate() {
        let other = (1.0 - t.work - t.wait - t.idle).max(0.0);
        println!(
            "  {:<9} {:>5.0}%  {:>9.0}%  {:>5.0}%  {:>5.0}%  {:>10.1}",
            profile::thread_name(i),
            t.work * 100.0,
            t.wait * 100.0,
            t.idle * 100.0,
            other * 100.0,
            t.jobs_per_frame
        );
    }

    println!("\n== memory ==");
    println!("  heap live {}   allocations per frame {:.1} ({:.1} KiB)", kib(last.heap_live_bytes), last.allocs_per_frame, last.alloc_kib_per_frame);
    let mut mem = last.memory.clone();
    mem.sort_by_key(|m| std::cmp::Reverse(m.1));
    for (name, bytes) in mem {
        println!("  {:<24} {:>10}", name, kib(bytes as u64));
    }
    let mut hot: Vec<&Pass> = last.passes.iter().filter(|p| p.allocs >= 1.0).collect();
    hot.sort_by(|a, b| b.allocs.total_cmp(&a.allocs));
    if !hot.is_empty() {
        println!("  allocating scopes (allocations per frame, nested scopes included):");
        for p in hot.iter().take(8) {
            println!("    {:<22} {:>8.1}", p.name, p.allocs);
        }
    }
    for (name, peak) in &last.counter_peaks {
        println!("  peak {:<30} {:>10.1}", name, peak);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_merges_overlaps() {
        assert_eq!(union_ns(&mut [(0, 10), (5, 15), (20, 30)]), 25);
        assert_eq!(union_ns(&mut []), 0);
    }

    #[test]
    fn cache_fit_picks_the_smallest_level() {
        let c = |level, bytes| Cache { level, kind: "data", bytes, line: 64, shared_by: 1 };
        let caches = [c(1, 32 << 10), c(2, 256 << 10), c(3, 8 << 20)];
        assert_eq!(fits_in(1000, &caches), "L1");
        assert_eq!(fits_in(100 << 10, &caches), "L2");
        assert_eq!(fits_in(4 << 20, &caches), "L3");
        assert_eq!(fits_in(64 << 20, &caches), "RAM");
    }

    fn ev(name: &'static str, kind: Kind, frame: u32, start: u64, dur: u64) -> Event {
        Event { name, kind, thread: 0, depth: 0, frame, start_ns: start, dur_ns: dur, bytes: 0, allocs: 0, queued_ns: 0 }
    }

    #[test]
    fn frame_life_follows_a_frame_through_the_pipeline() {
        // Frame 7 simulates at 0..4, renders during frame 8 (5..12) and is presented at 15..16.
        let events = [
            ev("tick", Kind::Schedule, 7, 0, 4),
            ev("extract", Kind::Phase, 7, 4, 1),
            ev("tick", Kind::Schedule, 8, 6, 4),
            ev("render", Kind::Job, 7, 5, 7),
            ev("present", Kind::Phase, 7, 15, 1),
        ];
        let lives = frame_lives(&events);
        let l7 = lives.iter().find(|l| l.frame == 7).unwrap();
        assert_eq!(l7.sim, (0, 4));
        assert_eq!(l7.render, Some((5, 12)));
        assert_eq!(l7.latency_ns(), Some(16));
        assert_eq!(lives.iter().find(|l| l.frame == 8).unwrap().latency_ns(), None);
    }
}
