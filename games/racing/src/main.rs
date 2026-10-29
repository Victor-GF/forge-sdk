//! Racing — M0: janela, loop, input, profiler, memória sem malloc e crash report.
//! Ainda sem render: velocidade e tempo de frame aparecem no título da janela.
//!
//! Controles: W acelera, S freia, Esc sai.
//! Ao sair, grava `racing.trace.json` (abrir em https://ui.perfetto.dev).
//!
//! Memória: um único `VirtualAlloc` com o orçamento abaixo; a thread principal usa a sua
//! `ThreadMemory` (persistent, level, frame). Depois da inicialização o alocador global é
//! travado: qualquer `Vec`/`String`/`Box` gera crash report e aborta.

use core::fmt::Write;
use std::process::ExitCode;

use forja_core::math;
use forja_core::text::TextBuf;
use forja_diag::crash::{self, CrashConfig};
use forja_diag::log;
use forja_inspect::{ChromeTraceExport, Counter, ENABLED as PROFILER_ENABLED, ProfEvent, Profiler, Zone};
use forja_mem::arena::Arena;
use forja_mem::global;
use forja_mem::region::{MemoryRegion, ThreadMemory, ThreadMemoryDesc};
use forja_platform::file::FileWriter;
use forja_platform::input::Key;
use forja_platform::{Platform, WindowDesc};

#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: forja_mem::global::GlobalAllocator = forja_mem::global::GlobalAllocator;

// ---------------------------------------------------------------- orçamento de memória

const MB: usize = 1024 * 1024;
const MAIN_THREAD_MEMORY: ThreadMemoryDesc =
    ThreadMemoryDesc { persistent: 32 * MB, level: 16 * MB, frame: 4 * MB };
/// Só a thread principal por enquanto; sobra para as arenas de outras threads no futuro.
const REGION_SIZE: usize = 64 * MB;
const REGION_SIZE_MB: usize = REGION_SIZE / MB;

// ---------------------------------------------------------------- profiler

const ZONE_FRAME: Zone = Zone(0);
const ZONE_PLATFORM: Zone = Zone(1);
const ZONE_UPDATE: Zone = Zone(2);
const ZONE_WAIT: Zone = Zone(3);
const COUNTER_FRAME_MEMORY: Counter = Counter(4);
const COUNTER_GLOBAL_MEMORY: Counter = Counter(5);
static PROFILER_NAMES: [&str; 6] = [
    "frame",
    "platform",
    "update",
    "wait",
    "frame memory (bytes)",
    "global memory (bytes)",
];

/// ~30 min a 60 fps com 10 eventos por frame (16 MB da arena persistent).
const PROFILER_CAPACITY: usize = 1 << 20;
const TRACE_PATH: &str = "racing.trace.json";

const TARGET_FRAME_SECONDS: f64 = 1.0 / 60.0;
/// Limite do dt da simulação (pausa no debugger, janela arrastada).
const MAX_DT: f32 = 0.1;

const KEY_THROTTLE: Key = Key(b'W');
const KEY_BRAKE: Key = Key(b'S');

// ---------------------------------------------------------------- carro (provisório)

const CAR_ACCELERATION: f32 = 12.0; // m/s²
const CAR_BRAKING: f32 = 25.0; // m/s²
const CAR_DRAG: f32 = 2.0; // m/s² sem acelerar
const CAR_MAX_SPEED: f32 = 83.0; // m/s (~300 km/h)

#[repr(C)]
struct Car {
    speed: f32, // m/s
}

impl Car {
    fn update(&mut self, throttle: bool, brake: bool, dt: f32) {
        let a = if brake {
            -CAR_BRAKING
        } else if throttle {
            CAR_ACCELERATION
        } else {
            -CAR_DRAG
        };
        self.speed = math::clamp(self.speed + a * dt, 0.0, CAR_MAX_SPEED);
    }
}

// ---------------------------------------------------------------- estatística do título

struct FrameStats {
    accumulated_s: f64,
    worst_s: f64,
    frames: u32,
}

impl FrameStats {
    const EMPTY: FrameStats = FrameStats { accumulated_s: 0.0, worst_s: 0.0, frames: 0 };

    fn add_frame(&mut self, dt: f32) {
        let dt = f64::from(dt);
        self.accumulated_s += dt;
        if dt > self.worst_s {
            self.worst_s = dt;
        }
        self.frames = self.frames.saturating_add(1);
    }

    /// A cada meio segundo, mostra média e pior frame no título. O texto vem da arena do frame.
    fn update_title(&mut self, platform: &Platform, car: &Car, frame_arena: &Arena) {
        if self.accumulated_s < 0.5 || self.frames == 0 {
            return;
        }
        let average_ms = self.accumulated_s / f64::from(self.frames) * 1000.0;
        let worst_ms = self.worst_s * 1000.0;
        let kmh = car.speed * 3.6;
        *self = FrameStats::EMPTY;

        let Some(buf) = frame_arena.alloc_slice(128, 0u8) else {
            return;
        };
        let mut t = TextBuf::new(buf);
        let _ = write!(t, "Forja racing | {average_ms:.2} ms (pior {worst_ms:.2}) | {kmh:.0} km/h");
        platform.set_title(t.as_str());
    }
}

// ---------------------------------------------------------------- main

fn main() -> ExitCode {
    crash::install(CrashConfig { app: "racing", version: env!("CARGO_PKG_VERSION"), dir: "crash" });

    let Some(mut region) = MemoryRegion::reserve(REGION_SIZE) else {
        log::error(format_args!("sem memória: o SO recusou {REGION_SIZE_MB} MB"));
        return ExitCode::FAILURE;
    };
    let Some(mut memory) = region.thread_memory(&MAIN_THREAD_MEMORY) else {
        log::error(format_args!("a memória da thread principal não cabe em {REGION_SIZE_MB} MB"));
        return ExitCode::FAILURE;
    };

    let capacity = if PROFILER_ENABLED { PROFILER_CAPACITY } else { 0 };
    let Some(events) = memory.persistent.alloc_slice(capacity, ProfEvent::EMPTY) else {
        memory.persistent.oom_fatal()
    };
    let mut prof = Profiler::new(&PROFILER_NAMES, events);

    let desc = WindowDesc { title: "Forja racing", width: 1280, height: 720 };
    let mut platform = match Platform::init(&desc) {
        Ok(p) => p,
        Err(e) => {
            log::error(format_args!("falha ao criar janela: {e:?}"));
            return ExitCode::FAILURE;
        }
    };

    let g = global::stats();
    log::info(format_args!(
        "inicialização: {} bytes em {} alocações globais (std); travando o alocador global",
        g.used, g.allocations
    ));
    global::lock();

    let mut car = Car { speed: 0.0 };
    let mut stats = FrameStats::EMPTY;

    loop {
        crash::set_frame(platform.frame_index);
        prof.begin(ZONE_FRAME);

        prof.begin(ZONE_PLATFORM);
        let running = platform.poll();
        prof.end(ZONE_PLATFORM);
        if !running || platform.input.key_pressed(Key::ESCAPE) {
            prof.end(ZONE_FRAME);
            break;
        }

        prof.begin(ZONE_UPDATE);
        let dt = platform.dt.min(MAX_DT);
        car.update(platform.input.key_down(KEY_THROTTLE), platform.input.key_down(KEY_BRAKE), dt);
        stats.add_frame(platform.dt);
        stats.update_title(&platform, &car, &memory.frame);
        prof.end(ZONE_UPDATE);

        prof.begin(ZONE_WAIT);
        platform.wait_frame(TARGET_FRAME_SECONDS);
        prof.end(ZONE_WAIT);

        end_frame(&mut prof, &mut memory.frame);
        prof.end(ZONE_FRAME);
    }

    platform.shutdown();
    save_trace(&prof);
    log_memory(&memory);
    ExitCode::SUCCESS
}

/// Registra o uso de memória do frame e zera a arena do frame.
fn end_frame(prof: &mut Profiler, frame_arena: &mut Arena) {
    prof.counter(COUNTER_FRAME_MEMORY, u32::try_from(frame_arena.used()).unwrap_or(u32::MAX));
    prof.counter(COUNTER_GLOBAL_MEMORY, u32::try_from(global::stats().used).unwrap_or(u32::MAX));
    frame_arena.reset();
}

/// Exporta em pedaços de 64 KB (buffer na stack) direto para o arquivo.
fn save_trace(prof: &Profiler) {
    if !PROFILER_ENABLED {
        return;
    }
    let Some(file) = FileWriter::create(TRACE_PATH) else {
        log::error(format_args!("falha ao criar {TRACE_PATH}"));
        return;
    };
    let mut export = ChromeTraceExport::new(prof.tsc_per_second());
    let mut buf = [0u8; 64 * 1024];
    let mut ok = true;
    loop {
        let n = export.next_chunk(prof, &mut buf);
        let Some(chunk) = buf.get(..n) else {
            break;
        };
        if n == 0 {
            break;
        }
        if !file.write_all(chunk) {
            ok = false;
            break;
        }
    }
    file.close();
    if ok {
        log::info(format_args!(
            "trace salvo em {TRACE_PATH} ({} eventos, {} descartados)",
            prof.used(),
            prof.dropped()
        ));
    } else {
        log::error(format_args!("falha ao gravar {TRACE_PATH}"));
    }
}

fn log_memory(memory: &ThreadMemory) {
    for a in [&memory.persistent, &memory.level, &memory.frame] {
        let s = a.stats();
        log::info(format_args!(
            "arena {:<11} pico {:>9} / {:>9} bytes, {} alocações, {} falhas",
            a.name(),
            s.peak,
            s.capacity,
            s.allocations,
            s.failures
        ));
    }
    let g = global::stats();
    log::info(format_args!(
        "global: {} / {} bytes, {} alocações, {} liberações (travado: {})",
        g.used, g.capacity, g.allocations, g.frees, g.locked
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn car_accelerates_brakes_and_respects_limits() {
        let mut c = Car { speed: 0.0 };
        c.update(true, false, 1.0);
        assert_eq!(c.speed, CAR_ACCELERATION);
        c.update(true, true, 1.0);
        assert_eq!(c.speed, 0.0);
        for _ in 0..100 {
            c.update(true, false, 1.0);
        }
        assert_eq!(c.speed, CAR_MAX_SPEED);
    }
}
