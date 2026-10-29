//! Profiler de zonas + flight recorder: guarda os últimos N frames inteiros
//! (cada sistema, cada sub-job, em qual lane) para a timeline do editor e
//! para anexar aos relatórios de falha.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

pub const NO_SYS: u16 = u16::MAX;
pub const NO_SUB: u16 = u16::MAX;
/// Valor de `tick` para zonas do grafo de render (que roda uma vez por frame).
pub const RENDER_TICK: u8 = u8::MAX;

#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub name: &'static str,
    pub sys: u16,
    pub sub: u16,
    pub lane: u8,
    pub tick: u8,
    pub start: u64,
    pub end: u64,
}

impl Zone {
    pub fn dur(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrameRec {
    pub index: u64,
    pub start: u64,
    pub end: u64,
    pub ticks: u8,
    pub timescale: f32,
    pub zones: Vec<Zone>,
}

impl FrameRec {
    pub fn dur(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

pub struct Profiler {
    cur: Mutex<Vec<Zone>>,
    ring: Mutex<VecDeque<FrameRec>>,
    frame_start: AtomicU64,
    frame_index: AtomicU64,
    capacity: usize,
}

static PROF: OnceLock<Profiler> = OnceLock::new();
static T0: OnceLock<Instant> = OnceLock::new();

pub fn get() -> &'static Profiler {
    PROF.get_or_init(|| Profiler {
        cur: Mutex::new(Vec::with_capacity(512)),
        ring: Mutex::new(VecDeque::new()),
        frame_start: AtomicU64::new(0),
        frame_index: AtomicU64::new(0),
        capacity: 600,
    })
}

pub fn now_ns() -> u64 {
    T0.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Profiler {
    pub fn begin_frame(&self) {
        self.frame_start.store(now_ns(), Ordering::Relaxed);
    }

    pub fn zone(&self, z: Zone) {
        lock(&self.cur).push(z);
    }

    pub fn end_frame(&self, ticks: u8, timescale: f32) -> u64 {
        let index = self.frame_index.fetch_add(1, Ordering::Relaxed);
        let mut zones = std::mem::take(&mut *lock(&self.cur));
        let mut ring = lock(&self.ring);
        // reaproveita a alocação do frame mais antigo
        let mut recycled = if ring.len() >= self.capacity { ring.pop_front().map(|f| f.zones).unwrap_or_default() } else { Vec::new() };
        recycled.clear();
        std::mem::swap(&mut zones, &mut recycled);
        *lock(&self.cur) = zones;
        ring.push_back(FrameRec {
            index,
            start: self.frame_start.load(Ordering::Relaxed),
            end: now_ns(),
            ticks,
            timescale,
            zones: recycled,
        });
        index
    }

    pub fn with_frames<R>(&self, f: impl FnOnce(&VecDeque<FrameRec>) -> R) -> R {
        f(&lock(&self.ring))
    }

    pub fn last_frame(&self) -> Option<FrameRec> {
        lock(&self.ring).back().cloned()
    }

    /// Exporta os últimos `seconds` em formato Chrome Trace (abre em ui.perfetto.dev).
    /// Com `try_only`, desiste em vez de bloquear (usado pelo handler de crash).
    pub fn export_chrome(&self, seconds: f64, try_only: bool, lane_names: &dyn Fn(usize) -> String) -> Option<String> {
        let ring = if try_only { self.ring.try_lock().ok()? } else { lock(&self.ring) };
        let last_end = ring.back().map(|f| f.end).unwrap_or(0);
        let from = last_end.saturating_sub((seconds * 1e9) as u64);
        let mut out = String::with_capacity(1 << 16);
        out.push_str("{\"traceEvents\":[\n");
        let mut lanes = 0usize;
        let mut first = true;
        for f in ring.iter().filter(|f| f.start >= from) {
            let _ = write!(
                out,
                "{}{{\"name\":\"frame {} ({} ticks, ts {:.2})\",\"ph\":\"X\",\"pid\":1,\"tid\":999,\"ts\":{:.3},\"dur\":{:.3}}}",
                if first { "" } else { ",\n" },
                f.index,
                f.ticks,
                f.timescale,
                f.start as f64 / 1e3,
                f.dur() as f64 / 1e3
            );
            first = false;
            for z in &f.zones {
                lanes = lanes.max(z.lane as usize + 1);
                let name = if z.sub != NO_SUB { format!("{}#{}", z.name, z.sub) } else { z.name.to_string() };
                let _ = write!(
                    out,
                    ",\n{{\"name\":\"{}\",\"ph\":\"X\",\"pid\":1,\"tid\":{},\"ts\":{:.3},\"dur\":{:.3},\"args\":{{\"tick\":{}}}}}",
                    name.replace('"', "'"),
                    z.lane,
                    z.start as f64 / 1e3,
                    z.dur() as f64 / 1e3,
                    z.tick
                );
            }
        }
        for l in 0..lanes {
            let _ = write!(out, ",\n{{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":{},\"args\":{{\"name\":\"{}\"}}}}", l, lane_names(l));
        }
        out.push_str(",\n{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":999,\"args\":{\"name\":\"frames\"}}");
        out.push_str("\n]}\n");
        Some(out)
    }
}

/// Zona ad-hoc com RAII: `let _z = profiler::scope("cook.track");`
pub struct Scoped {
    name: &'static str,
    start: u64,
}

pub fn scope(name: &'static str) -> Scoped {
    Scoped { name, start: now_ns() }
}

impl Drop for Scoped {
    fn drop(&mut self) {
        get().zone(Zone {
            name: self.name,
            sys: NO_SYS,
            sub: NO_SUB,
            lane: crate::jobs::lane() as u8,
            tick: 0,
            start: self.start,
            end: now_ns(),
        });
    }
}
