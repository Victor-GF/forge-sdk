//! Lightweight instrumentation: scopes recorded per thread, exportable as a Chrome/Perfetto trace
//! (open ui.perfetto.dev).
//!
//! Every scope has a **kind**, the level of the engine it belongs to, so a trace reads from the
//! top down:
//!
//! | kind       | what it is                                   | recorded by                    |
//! |------------|----------------------------------------------|--------------------------------|
//! | `Phase`    | a step of the frame: extract, present, ...   | `frame::FramePipeline`         |
//! | `Schedule` | one run of a schedule ("frame", "tick")      | `schedule::Schedule::run`      |
//! | `System`   | one system                                   | the schedule's executor        |
//! | `Pass`     | a named piece of work inside a system        | `scope` / `scope_bytes`        |
//! | `Job`      | a job taken from the queue by some thread    | `jobs` (named after its spawner)|
//! | `Wait`     | waiting for jobs, helping when allowed       | `jobs::help_while`             |
//! | `Idle`     | a worker asleep: nothing queued              | `jobs` worker loop             |
//!
//! Each event also carries the **frame** it works for, which is not always the frame being
//! simulated: the render job of frame N runs during frame N+1. The tag is set by the schedule and
//! the frame pipeline and inherited by jobs, like the label (the helpers of a `par_for` inside
//! "physics.contacts" record jobs named "physics.contacts").
//!
//! **Flows** link a job to the scope that spawned it, and a joined job to the waiter that took its
//! result. Perfetto draws them as arrows: the schedule's dependency graph, the fan-out of a
//! `par_for` and the hand-off from simulation to render all show up without extra code.
//!
//! Besides time, a scope carries the bytes its pass touches (`scope_bytes`, declared by the code
//! that knows its data layout) and the allocations made on its thread while it was open (when the
//! binary installs `alloc::CountingAlloc`). Jobs carry how long they sat in the queue.

use std::{
    cell::Cell,
    io::{self, Write},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Phase,
    Schedule,
    System,
    Pass,
    Job,
    Wait,
    Idle,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Phase => "phase",
            Kind::Schedule => "schedule",
            Kind::System => "system",
            Kind::Pass => "pass",
            Kind::Job => "job",
            Kind::Wait => "wait",
            Kind::Idle => "idle",
        }
    }

    /// Doing something, as opposed to waiting or sleeping.
    pub fn is_work(self) -> bool {
        !matches!(self, Kind::Wait | Kind::Idle)
    }
}

/// Frame tag of work that belongs to no frame (startup, a job spawned outside the frame loop).
pub const NO_FRAME: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub name: &'static str,
    pub kind: Kind,
    pub thread: u16,
    pub depth: u16,
    /// Frame this work belongs to, or `NO_FRAME`.
    pub frame: u32,
    pub start_ns: u64,
    pub dur_ns: u64,
    /// Bytes the pass reads and writes, if the scope declared them (`scope_bytes`).
    pub bytes: u64,
    /// Allocations made on this thread while the scope was open, nested scopes included.
    pub allocs: u32,
    /// Jobs only: time between being queued and starting.
    pub queued_ns: u32,
}

/// A value over time, e.g. live heap bytes once per frame.
#[derive(Clone, Copy, Debug)]
pub struct Counter {
    pub name: &'static str,
    pub ts_ns: u64,
    pub value: f64,
}

/// One end of an arrow between two slices: `out` on the source, `in` on the target.
#[derive(Clone, Copy, Debug)]
pub struct Flow {
    pub id: u64,
    pub thread: u16,
    pub ts_ns: u64,
    pub out: bool,
}

/// Name of the scope recorded while a thread waits for other jobs (helping when it can).
pub const WAIT: &str = "(wait)";
/// Name of the scope recorded while a worker sleeps because both queues are empty.
pub const IDLE: &str = "(idle)";

static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: OnceLock<Instant> = OnceLock::new();
static SINKS: Mutex<Vec<Arc<Mutex<Vec<Event>>>>> = Mutex::new(Vec::new());
static COUNTERS: Mutex<Vec<Counter>> = Mutex::new(Vec::new());
static FLOWS: Mutex<Vec<Flow>> = Mutex::new(Vec::new());
static NEXT_FLOW: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static LOCAL: Arc<Mutex<Vec<Event>>> = {
        let sink = Arc::new(Mutex::new(Vec::with_capacity(4096)));
        SINKS.lock().unwrap().push(sink.clone());
        sink
    };
    static LABEL: Cell<&'static str> = const { Cell::new("") };
    static DEPTH: Cell<u16> = const { Cell::new(0) };
    static FRAME: Cell<u32> = const { Cell::new(NO_FRAME) };
}

pub fn set_enabled(on: bool) {
    EPOCH.get_or_init(Instant::now);
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn now_ns() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// Name of the innermost open scope on this thread ("" if none).
pub fn current_label() -> &'static str {
    LABEL.with(|l| l.get())
}

/// Frame tag of the work running on this thread (`NO_FRAME` if none).
pub fn current_frame() -> u32 {
    FRAME.with(|f| f.get())
}

/// Tags work on this thread as belonging to `frame` until the guard drops.
#[must_use]
pub struct FrameTag(u32);

pub fn frame_tag(frame: u64) -> FrameTag {
    FrameTag(FRAME.with(|f| f.replace(frame as u32)))
}

impl Drop for FrameTag {
    fn drop(&mut self) {
        FRAME.with(|f| f.set(self.0));
    }
}

#[must_use]
pub struct Scope {
    name: &'static str,
    kind: Kind,
    start: u64,
    prev_label: &'static str,
    active: bool,
    allocs: u64,
    bytes: u64,
    queued_ns: u64,
    /// Jobs set the frame tag of their spawner for their duration; this is the tag to restore.
    prev_frame: Option<u32>,
}

const INACTIVE: Scope =
    Scope { name: "", kind: Kind::Pass, start: 0, prev_label: "", active: false, allocs: 0, bytes: 0, queued_ns: 0, prev_frame: None };

/// Records the time until the returned guard is dropped. `WAIT` and `IDLE` get their own kinds;
/// any other name is a `Pass`.
pub fn scope(name: &'static str) -> Scope {
    let kind = match name {
        WAIT => Kind::Wait,
        IDLE => Kind::Idle,
        _ => Kind::Pass,
    };
    scope_kind(kind, name)
}

/// Like `scope`, also recording how many bytes the pass reads and writes. Used to compare a
/// pass's footprint with the cache sizes and to compute its bandwidth.
pub fn scope_bytes(name: &'static str, bytes: usize) -> Scope {
    let mut s = scope_kind(Kind::Pass, name);
    s.bytes = bytes as u64;
    s
}

pub fn scope_kind(kind: Kind, name: &'static str) -> Scope {
    if !enabled() {
        return INACTIVE;
    }
    let prev_label = LABEL.with(|l| l.replace(name));
    DEPTH.with(|d| d.set(d.get() + 1));
    Scope {
        name,
        kind,
        start: now_ns(),
        prev_label,
        active: true,
        allocs: crate::alloc::thread_allocs(),
        bytes: 0,
        queued_ns: 0,
        prev_frame: None,
    }
}

/// Scope of a job taken from the queue: named after its spawner, working for `frame`.
pub fn job_scope(name: &'static str, frame: u32, queued_ns: u64) -> Scope {
    if !enabled() {
        return INACTIVE;
    }
    let mut s = scope_kind(Kind::Job, if name.is_empty() { "job" } else { name });
    s.queued_ns = queued_ns;
    s.prev_frame = Some(FRAME.with(|f| f.replace(frame)));
    s
}

impl Scope {
    /// Sets the bytes the pass touched, when they are only known at the end.
    pub fn set_bytes(&mut self, bytes: usize) {
        self.bytes = bytes as u64;
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let end = now_ns();
        let depth = DEPTH.with(|d| {
            d.set(d.get() - 1);
            d.get()
        });
        LABEL.with(|l| l.set(self.prev_label));
        let event = Event {
            name: self.name,
            kind: self.kind,
            thread: crate::jobs::thread_index() as u16,
            depth,
            frame: current_frame(),
            start_ns: self.start,
            dur_ns: end - self.start,
            bytes: self.bytes,
            allocs: (crate::alloc::thread_allocs() - self.allocs) as u32,
            queued_ns: self.queued_ns.min(u32::MAX as u64) as u32,
        };
        if let Some(prev) = self.prev_frame {
            FRAME.with(|f| f.set(prev));
        }
        LOCAL.with(|l| l.lock().unwrap().push(event));
    }
}

/// Records `value` for the counter track `name` at the current time.
pub fn counter(name: &'static str, value: f64) {
    if enabled() {
        COUNTERS.lock().unwrap().push(Counter { name, ts_ns: now_ns(), value });
    }
}

/// Starts an arrow at the scope open on this thread. Returns its id (0 when not recording).
pub fn flow_out() -> u64 {
    if !enabled() {
        return 0;
    }
    let id = NEXT_FLOW.fetch_add(1, Ordering::Relaxed);
    let flow = Flow { id, thread: crate::jobs::thread_index() as u16, ts_ns: now_ns(), out: true };
    FLOWS.lock().unwrap().push(flow);
    id
}

/// Ends arrow `id` at the scope open on this thread.
pub fn flow_in(id: u64) {
    if id != 0 && enabled() {
        let flow = Flow { id, thread: crate::jobs::thread_index() as u16, ts_ns: now_ns(), out: false };
        FLOWS.lock().unwrap().push(flow);
    }
}

/// Drains the events recorded by every thread, sorted by start time.
pub fn take_events() -> Vec<Event> {
    let mut out = Vec::new();
    for sink in SINKS.lock().unwrap().iter() {
        out.append(&mut sink.lock().unwrap());
    }
    out.sort_by_key(|e| e.start_ns);
    out
}

/// Drains the recorded counter samples, in time order.
pub fn take_counters() -> Vec<Counter> {
    let mut out = std::mem::take(&mut *COUNTERS.lock().unwrap());
    out.sort_by_key(|c| c.ts_ns);
    out
}

/// Drains the recorded flow ends. Only arrows with both ends are kept.
pub fn take_flows() -> Vec<Flow> {
    let mut out = std::mem::take(&mut *FLOWS.lock().unwrap());
    let mut ins: Vec<u64> = out.iter().filter(|f| !f.out).map(|f| f.id).collect();
    ins.sort_unstable();
    let mut outs: Vec<u64> = out.iter().filter(|f| f.out).map(|f| f.id).collect();
    outs.sort_unstable();
    out.retain(|f| ins.binary_search(&f.id).is_ok() && outs.binary_search(&f.id).is_ok());
    out.sort_by_key(|f| f.ts_ns);
    out
}

/// Self time of each event: its duration minus the events nested in it on the same thread.
pub fn self_times(events: &[Event]) -> Vec<u64> {
    // Walk each thread's events in start order with a stack of open scopes and subtract every
    // child from its parent.
    let mut self_ns: Vec<u64> = events.iter().map(|e| e.dur_ns).collect();
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&i| (events[i].thread, events[i].start_ns, std::cmp::Reverse(events[i].dur_ns)));
    let mut stack: Vec<usize> = Vec::new();
    let mut thread = u16::MAX;
    for &i in &order {
        let e = &events[i];
        if e.thread != thread {
            stack.clear();
            thread = e.thread;
        }
        while let Some(&top) = stack.last() {
            if events[top].start_ns + events[top].dur_ns <= e.start_ns {
                stack.pop();
            } else {
                break;
            }
        }
        if let Some(&parent) = stack.last() {
            self_ns[parent] = self_ns[parent].saturating_sub(e.dur_ns);
        }
        stack.push(i);
    }
    self_ns
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub wall_ns: u64,
    /// Time each thread spent doing recorded work (self time of every scope except waits).
    pub busy_ns: Vec<u64>,
    /// (name, self ns, count), sorted by self time. Self time excludes nested scopes on the same
    /// thread, so a scope that waited while helping with other work isn't charged for that work.
    pub by_name: Vec<(&'static str, u64, u32)>,
}

impl Report {
    pub fn summarize(events: &[Event], threads: usize) -> Report {
        let (Some(first), Some(last)) = (events.iter().map(|e| e.start_ns).min(), events.iter().map(|e| e.start_ns + e.dur_ns).max())
        else {
            return Report { busy_ns: vec![0; threads], ..Report::default() };
        };
        let mut busy_ns = vec![0u64; threads];
        let self_ns = self_times(events);
        let mut by_name: Vec<(&'static str, u64, u32)> = Vec::new();
        for (e, &ns) in events.iter().zip(&self_ns) {
            if e.kind.is_work() && (e.thread as usize) < threads {
                busy_ns[e.thread as usize] += ns;
            }
            match by_name.iter_mut().find(|(n, _, _)| *n == e.name) {
                Some(entry) => {
                    entry.1 += ns;
                    entry.2 += 1;
                }
                None => by_name.push((e.name, ns, 1)),
            }
        }
        by_name.sort_by_key(|e| std::cmp::Reverse(e.1));
        Report { wall_ns: last - first, busy_ns, by_name }
    }

    /// Fraction of available thread time spent doing recorded work, 0..1.
    pub fn utilization(&self) -> f32 {
        if self.wall_ns == 0 || self.busy_ns.is_empty() {
            return 0.0;
        }
        let busy: u64 = self.busy_ns.iter().sum();
        busy as f32 / (self.wall_ns as f32 * self.busy_ns.len() as f32)
    }

    pub fn print(&self, frames: u32) {
        let frames = frames.max(1) as f64;
        println!(
            "  utilization {:5.1}%  ({} threads, wall {:.2} ms/frame)",
            self.utilization() * 100.0,
            self.busy_ns.len(),
            self.wall_ns as f64 / 1e6 / frames
        );
        println!("  self time, summed over threads:");
        for (name, total, count) in self.by_name.iter().take(14) {
            println!("  {:>9.3} ms/frame  {:>6} scopes  {}", *total as f64 / 1e6 / frames, count, name);
        }
    }
}

/// One frame of a recorded run.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSpan {
    pub index: u64,
    pub start_ns: u64,
    pub end_ns: u64,
    pub ticks: u32,
    /// Heap allocations during the frame, and their bytes.
    pub allocs: u64,
    pub alloc_bytes: u64,
    pub live_bytes: u64,
    /// Jobs executed by all threads during the frame.
    pub jobs: u64,
}

/// A slice on a named track of its own, above the thread tracks. Used for the frame pipeline:
/// one track per stage, where frame N renders while frame N+1 simulates. Slices on one track
/// must not overlap.
#[derive(Clone, Debug)]
pub struct Span {
    pub track: &'static str,
    pub name: String,
    pub start_ns: u64,
    pub end_ns: u64,
    pub args: Vec<(&'static str, f64)>,
}

/// One process in a trace file: a run of the engine with its own threads and counters.
pub struct TraceRun<'a> {
    pub pid: u32,
    pub name: String,
    pub events: &'a [Event],
    pub counters: &'a [Counter],
    pub flows: &'a [Flow],
    pub spans: &'a [Span],
    pub thread_names: Vec<String>,
    /// Drawn as slices "frame N" on their own track, with the frame's stats as args.
    pub frames: &'a [FrameSpan],
}

/// Track id used for the frame slices.
const FRAME_TRACK: usize = 999;
/// First track id used for `Span` tracks.
const SPAN_TRACKS: usize = 1000;

/// Writes a single run as a Chrome/Perfetto JSON trace.
pub fn write_chrome_trace(path: &std::path::Path, events: &[Event]) -> io::Result<()> {
    let threads = events.iter().map(|e| e.thread as usize + 1).max().unwrap_or(0);
    let run = TraceRun {
        pid: 0,
        name: "forja".into(),
        events,
        counters: &[],
        flows: &[],
        spans: &[],
        thread_names: (0..threads).map(thread_name).collect(),
        frames: &[],
    };
    write_trace(path, &[run])
}

pub fn thread_name(i: usize) -> String {
    if i == 0 { "main".into() } else { format!("worker {i}") }
}

/// Writes several runs into one Chrome/Perfetto JSON trace, each as its own process with time
/// starting at 0, so their timelines line up for comparison. Slices are categorized by kind and
/// carry `frame`, `bytes`, `KiB`, `GBps`, `allocs` and, for jobs, `queued_us` as args.
pub fn write_trace(path: &std::path::Path, runs: &[TraceRun]) -> io::Result<()> {
    let mut out = io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "{{\"displayTimeUnit\":\"ms\",\"traceEvents\":[")?;
    let mut first = true;
    let mut sep = |out: &mut io::BufWriter<std::fs::File>| -> io::Result<()> {
        if !std::mem::take(&mut first) {
            write!(out, ",")?;
        }
        writeln!(out)
    };
    for run in runs {
        let pid = run.pid;
        let t0 = run
            .events
            .iter()
            .map(|e| e.start_ns)
            .chain(run.counters.iter().map(|c| c.ts_ns))
            .chain(run.frames.iter().map(|f| f.start_ns))
            .min()
            .unwrap_or(0);
        let us = |ns: u64| (ns.saturating_sub(t0)) as f64 / 1000.0;
        sep(&mut out)?;
        write!(out, "{{\"name\":\"process_name\",\"ph\":\"M\",\"pid\":{pid},\"args\":{{\"name\":\"{}\"}}}}", run.name)?;
        sep(&mut out)?;
        write!(out, "{{\"name\":\"process_sort_index\",\"ph\":\"M\",\"pid\":{pid},\"args\":{{\"sort_index\":{pid}}}}}")?;
        for (tid, name) in run.thread_names.iter().enumerate() {
            sep(&mut out)?;
            write!(out, "{{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":{pid},\"tid\":{tid},\"args\":{{\"name\":\"{name}\"}}}}")?;
            sep(&mut out)?;
            write!(out, "{{\"name\":\"thread_sort_index\",\"ph\":\"M\",\"pid\":{pid},\"tid\":{tid},\"args\":{{\"sort_index\":{tid}}}}}")?;
        }
        for e in run.events {
            sep(&mut out)?;
            write!(
                out,
                "{{\"name\":\"{}\",\"cat\":\"{}\",\"ph\":\"X\",\"pid\":{pid},\"tid\":{},\"ts\":{:.3},\"dur\":{:.3},\"args\":{{\"kind\":\"{}\"",
                e.name,
                e.kind.label(),
                e.thread,
                us(e.start_ns),
                e.dur_ns as f64 / 1000.0,
                e.kind.label()
            )?;
            if e.frame != NO_FRAME {
                write!(out, ",\"frame\":{}", e.frame)?;
            }
            if e.allocs > 0 {
                write!(out, ",\"allocs\":{}", e.allocs)?;
            }
            if e.bytes > 0 {
                let gbps = e.bytes as f64 / e.dur_ns.max(1) as f64;
                write!(out, ",\"bytes\":{},\"KiB\":{:.1},\"GBps\":{:.2}", e.bytes, e.bytes as f64 / 1024.0, gbps)?;
            }
            if e.kind == Kind::Job {
                write!(out, ",\"queued_us\":{:.1}", e.queued_ns as f64 / 1000.0)?;
            }
            write!(out, "}}}}")?;
        }
        for f in run.flows {
            sep(&mut out)?;
            // "s" binds to the slice open at ts on that thread; "f" with bp=e to the slice that
            // encloses ts (the job that just started).
            let (ph, bp) = if f.out { ("s", "") } else { ("f", ",\"bp\":\"e\"") };
            write!(
                out,
                "{{\"name\":\"spawn\",\"cat\":\"flow\",\"ph\":\"{ph}\",\"id\":{},\"pid\":{pid},\"tid\":{},\"ts\":{:.3}{bp}}}",
                f.id,
                f.thread,
                us(f.ts_ns)
            )?;
        }
        for c in run.counters {
            sep(&mut out)?;
            write!(
                out,
                "{{\"name\":\"{}\",\"ph\":\"C\",\"pid\":{pid},\"ts\":{:.3},\"args\":{{\"value\":{}}}}}",
                c.name,
                us(c.ts_ns),
                c.value
            )?;
        }
        // Each span track becomes a pseudo-thread above the real ones, in name order.
        let mut tracks: Vec<&'static str> = run.spans.iter().map(|s| s.track).collect();
        tracks.sort_unstable();
        tracks.dedup();
        for (k, track) in tracks.iter().enumerate() {
            let tid = SPAN_TRACKS + k;
            sep(&mut out)?;
            write!(out, "{{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":{pid},\"tid\":{tid},\"args\":{{\"name\":\"{track}\"}}}}")?;
            sep(&mut out)?;
            write!(out, "{{\"name\":\"thread_sort_index\",\"ph\":\"M\",\"pid\":{pid},\"tid\":{tid},\"args\":{{\"sort_index\":{}}}}}", k as i64 - 100)?;
        }
        for s in run.spans {
            let tid = SPAN_TRACKS + tracks.iter().position(|t| *t == s.track).unwrap_or(0);
            let args: Vec<String> = s.args.iter().map(|(k, v)| format!("\"{k}\":{v:.3}")).collect();
            sep(&mut out)?;
            write!(
                out,
                "{{\"name\":\"{}\",\"cat\":\"pipeline\",\"ph\":\"X\",\"pid\":{pid},\"tid\":{tid},\"ts\":{:.3},\"dur\":{:.3},\"args\":{{{}}}}}",
                s.name,
                us(s.start_ns),
                (s.end_ns - s.start_ns) as f64 / 1000.0,
                args.join(",")
            )?;
        }
        if !run.frames.is_empty() {
            sep(&mut out)?;
            write!(out, "{{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":{pid},\"tid\":{FRAME_TRACK},\"args\":{{\"name\":\"frames\"}}}}")?;
            sep(&mut out)?;
            write!(out, "{{\"name\":\"thread_sort_index\",\"ph\":\"M\",\"pid\":{pid},\"tid\":{FRAME_TRACK},\"args\":{{\"sort_index\":-1}}}}")?;
        }
        for f in run.frames {
            sep(&mut out)?;
            write!(
                out,
                "{{\"name\":\"frame {}\",\"cat\":\"frames\",\"ph\":\"X\",\"pid\":{pid},\"tid\":{FRAME_TRACK},\"ts\":{:.3},\"dur\":{:.3},\"args\":{{\"ms\":{:.3},\"ticks\":{},\"allocs\":{},\"alloc_KiB\":{:.1},\"heap_live_MiB\":{:.2},\"jobs\":{}}}}}",
                f.index,
                us(f.start_ns),
                (f.end_ns - f.start_ns) as f64 / 1000.0,
                (f.end_ns - f.start_ns) as f64 / 1e6,
                f.ticks,
                f.allocs,
                f.alloc_bytes as f64 / 1024.0,
                f.live_bytes as f64 / (1024.0 * 1024.0),
                f.jobs
            )?;
        }
    }
    writeln!(out, "\n]}}")?;
    out.flush()
}
