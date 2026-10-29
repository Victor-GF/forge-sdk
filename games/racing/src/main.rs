//! Racing — M0: janela, loop, input, profiler, memória sem malloc e crash report.
//! Ainda sem render: velocidade e tempo de frame aparecem no título da janela.
//!
//! Controles: W acelera, S freia, Esc sai.
//! Ao sair, grava `racing.trace.json` (abrir em https://ui.perfetto.dev).
//!
//! `racing --headless N`: sem janela (em qualquer SO), roda N frames o mais rápido possível com
//! dt fixo e entrada roteirizada. Serve para testes, depuração (gdb) e profiling (cachegrind,
//! perf). No Linux, que ainda não tem janela, é o único modo.
//!
//! Memória: uma única reserva ao SO com o orçamento abaixo; a thread principal usa a sua
//! `ThreadMemory` (persistent, level, frame). O heap geral (std/dependências) tem orçamento
//! próprio e, depois da inicialização, cada ponto de chamada que alocar aparece no log com a
//! pilha. A meta do jogo é zero alocações no heap geral por frame (contador no trace).

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
use forja_os::file::FileWriter;
use forja_platform::input::Key;
use forja_platform::{Platform, PlatformError, WindowDesc};

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
/// Heap geral: só std e dependências. Hoje o jogo usa 0 bytes; o orçamento é o alarme.
const HEAP_BUDGET: usize = MB;

// ---------------------------------------------------------------- profiler

const ZONE_FRAME: Zone = Zone(0);
const ZONE_PLATFORM: Zone = Zone(1);
const ZONE_UPDATE: Zone = Zone(2);
const ZONE_WAIT: Zone = Zone(3);
const COUNTER_FRAME_MEMORY: Counter = Counter(4);
const COUNTER_HEAP_LIVE: Counter = Counter(5);
const COUNTER_HEAP_ALLOCATIONS: Counter = Counter(6);
static PROFILER_NAMES: [&str; 7] = [
    "frame",
    "platform",
    "update",
    "wait",
    "frame arena (bytes)",
    "heap live (bytes)",
    "heap allocations (per frame)",
];

/// ~30 min a 60 fps com 10 eventos por frame (16 MB da arena persistent).
const PROFILER_CAPACITY: usize = 1 << 20;
const TRACE_PATH: &str = "racing.trace.json";

const TARGET_FRAME_SECONDS: f64 = 1.0 / 60.0;
/// Limite do dt da simulação (pausa no debugger, janela arrastada).
const MAX_DT: f32 = 0.1;
/// Modo headless: dt fixo, para a simulação dar o mesmo resultado em qualquer máquina.
const HEADLESS_DT: f32 = 1.0 / 60.0;
const HEADLESS_DEFAULT_FRAMES: u64 = 600;
/// Entrada roteirizada do modo headless: acelera por 4 s, freia por 1 s, repete.
const SCRIPT_PERIOD_FRAMES: u64 = 300;
const SCRIPT_THROTTLE_FRAMES: u64 = 240;

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
    global::set_budget(HEAP_BUDGET);

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

    let mut platform = match headless_frames_from_args() {
        Some(frames) => {
            log::info(format_args!("modo headless: {frames} frames, dt fixo, entrada roteirizada"));
            Platform::headless(frames)
        }
        None => {
            let desc = WindowDesc { title: "Forja racing", width: 1280, height: 720 };
            match Platform::init(&desc) {
                Ok(p) => p,
                Err(PlatformError::WindowUnsupported) => {
                    log::error(format_args!("este SO ainda não tem janela; rode com --headless N"));
                    return ExitCode::FAILURE;
                }
                Err(e) => {
                    log::error(format_args!("falha ao criar janela: {e:?}"));
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    let h = global::stats();
    log::info(format_args!(
        "inicialização: {} bytes vivos no heap geral em {} alocações; a partir daqui cada \
         ponto de chamada que alocar vai para o log",
        h.live_bytes, h.allocations
    ));
    global::end_startup();

    let mut car = Car { speed: 0.0 };
    let mut stats = FrameStats::EMPTY;
    let mut heap_allocations_before = global::stats().allocations;

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
        if platform.is_headless() {
            let in_period = platform.frame_index.wrapping_rem(SCRIPT_PERIOD_FRAMES);
            let throttle = in_period < SCRIPT_THROTTLE_FRAMES;
            car.update(throttle, !throttle, HEADLESS_DT);
        } else {
            let dt = platform.dt.min(MAX_DT);
            car.update(platform.input.key_down(KEY_THROTTLE), platform.input.key_down(KEY_BRAKE), dt);
        }
        stats.add_frame(platform.dt);
        stats.update_title(&platform, &car, &memory.frame);
        prof.end(ZONE_UPDATE);

        // Headless roda o mais rápido possível (profiling); com janela, trava em 60 fps.
        if !platform.is_headless() {
            prof.begin(ZONE_WAIT);
            platform.wait_frame(TARGET_FRAME_SECONDS);
            prof.end(ZONE_WAIT);
        }

        end_frame(&mut prof, &mut memory.frame, &mut heap_allocations_before);
        prof.end(ZONE_FRAME);
    }

    log::info(format_args!(
        "fim: {} frames, velocidade final {:.0} km/h",
        platform.frame_index.saturating_sub(1),
        car.speed * 3.6
    ));
    platform.shutdown();
    save_trace(&prof);
    log_memory(&memory);
    ExitCode::SUCCESS
}

/// `--headless N` (N opcional). Lido na inicialização: a std aloca para os argumentos, o que é
/// permitido antes de `global::end_startup()`.
fn headless_frames_from_args() -> Option<u64> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--headless" {
            let frames = args.next().and_then(|n| n.parse::<u64>().ok());
            return Some(frames.unwrap_or(HEADLESS_DEFAULT_FRAMES));
        }
    }
    None
}

/// Registra o uso de memória do frame e zera a arena do frame.
fn end_frame(prof: &mut Profiler, frame_arena: &mut Arena, heap_allocations_before: &mut u64) {
    let heap = global::stats();
    let heap_allocations = heap.allocations.saturating_sub(*heap_allocations_before);
    *heap_allocations_before = heap.allocations;
    prof.counter(COUNTER_FRAME_MEMORY, u32::try_from(frame_arena.used()).unwrap_or(u32::MAX));
    prof.counter(COUNTER_HEAP_LIVE, u32::try_from(heap.live_bytes).unwrap_or(u32::MAX));
    prof.counter(COUNTER_HEAP_ALLOCATIONS, u32::try_from(heap_allocations).unwrap_or(u32::MAX));
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
    let h = global::stats();
    log::info(format_args!(
        "heap geral: {} bytes vivos (pico {}, orçamento {}), {} alocações, {} liberações, \
         {} pontos de chamada relatados",
        h.live_bytes, h.peak_bytes, h.budget, h.allocations, h.frees, h.reported_sites
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
