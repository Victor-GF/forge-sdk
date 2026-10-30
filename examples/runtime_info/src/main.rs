//! Runtime info: exemplo que abre uma janela e mostra no terminal tudo que o runtime sabe sobre
//! o sistema e sobre si mesmo: SO, páginas, relógio, memória da máquina e do processo, região e
//! arenas da forja, heap geral por tag, janela, tempo de frame e input.
//!
//! Faz o papel do jogo (dono do `main` e do loop), mas sem jogo: serve para ver a infraestrutura
//! da engine funcionando de verdade.
//!
//! Controles (com a janela em foco):
//! - F1: imprime o relatório de novo (com o input daquele instante)
//! - F2: aloca no heap geral de propósito; o log mostra o ponto de chamada com a pilha
//! - F9: erro fatal de propósito (`crash::fatal`); grava o crash report em `crash/`
//! - Esc ou fechar a janela: sai, imprimindo o relatório final
//!
//! O título da janela mostra fps, tempo de frame e alocações do heap geral por frame.
//!
//! `runtime_info --headless N`: sem janela, roda N frames e imprime os relatórios (máquinas sem
//! servidor gráfico).
//!
//! Memória: como no jogo, uma reserva ao SO e a `ThreadMemory` da thread principal. O histórico
//! de frames fica na arena `persistent`; título e relatório saem da `frame`.

use core::fmt::{self, Write};
use std::process::ExitCode;

use forja_core::text::TextBuf;
use forja_diag::crash::{self, CrashConfig};
use forja_diag::log;
use forja_mem::arena::Arena;
use forja_mem::global::{self, AllocTag, HeapStats};
use forja_mem::region::{MemoryRegion, ThreadMemoryDesc, page_info};
use forja_mem::stats::{self, ARENAS_MAX};
use forja_os::{process, time};
use forja_platform::input::{Input, Key, MouseButton};
use forja_platform::{Platform, PlatformError, WindowDesc};

#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: forja_mem::global::GlobalAllocator = forja_mem::global::GlobalAllocator;

// ---------------------------------------------------------------- orçamento de memória

const KB: usize = 1024;
const MB: usize = 1024 * KB;
const GB: u64 = 1024 * 1024 * 1024;
const MAIN_THREAD_MEMORY: ThreadMemoryDesc =
    ThreadMemoryDesc { persistent: MB, level: MB, frame: 256 * KB };
const REGION_SIZE: usize = 8 * MB;
const REGION_SIZE_MB: usize = REGION_SIZE / MB;
/// Heap geral: só std e dependências. O exemplo não usa nada depois da inicialização (exceto F2).
const HEAP_BUDGET: usize = MB;

/// Texto do relatório, recortado da arena do frame a cada impressão.
const REPORT_CAPACITY: usize = 16 * KB;
const TITLE_CAPACITY: usize = 256;

// ---------------------------------------------------------------- frame e controles

const TARGET_FRAME_SECONDS: f64 = 1.0 / 60.0;
/// 2 s a 60 fps.
const FRAME_HISTORY: usize = 120;
const TITLE_INTERVAL_SECONDS: f64 = 0.5;
const HEADLESS_DEFAULT_FRAMES: u64 = 120;

const KEY_REPORT: Key = Key::F1;
const KEY_HEAP_ALLOC: Key = Key::F2;
const KEY_CRASH: Key = Key::F9;
const HEAP_DEMO_BYTES: usize = 4 * KB;

// ---------------------------------------------------------------- estado

/// O que não muda depois da inicialização.
struct Startup {
    start_ticks: u64,
    /// 0 = a std não soube dizer.
    logical_cpus: usize,
    main_thread_id: u64,
    /// Heap geral no fim da inicialização (antes de `global::end_startup`).
    heap: HeapStats,
}

/// Tempos dos últimos frames: ring buffer na arena `persistent`.
struct FrameHistory<'a> {
    dts: &'a mut [f32],
    next: usize,
    filled: usize,
}

#[derive(Clone, Copy)]
struct FrameSummary {
    frames: usize,
    average_s: f64,
    worst_s: f64,
}

impl FrameHistory<'_> {
    fn add(&mut self, dt: f32) {
        if let Some(slot) = self.dts.get_mut(self.next) {
            *slot = dt;
        }
        self.next = self.next.saturating_add(1);
        if self.next >= self.dts.len() {
            self.next = 0;
        }
        self.filled = self.filled.saturating_add(1).min(self.dts.len());
    }

    /// Média e pior frame dos tempos guardados (todos, quando o ring já deu a volta).
    fn summary(&self) -> FrameSummary {
        let mut total_s = 0.0;
        let mut worst_s = 0.0;
        for dt in self.dts.iter().take(self.filled) {
            let dt = f64::from(*dt);
            total_s += dt;
            if dt > worst_s {
                worst_s = dt;
            }
        }
        let average_s = if self.filled == 0 { 0.0 } else { total_s / self.filled as f64 };
        FrameSummary { frames: self.filled, average_s, worst_s }
    }
}

/// O que o relatório lê. Tudo emprestado: o relatório não guarda nada.
struct ReportInput<'a> {
    startup: &'a Startup,
    region: &'a MemoryRegion,
    platform: &'a Platform,
    history: &'a FrameHistory<'a>,
    heap_allocations_last_frame: u64,
}

// ---------------------------------------------------------------- main

fn main() -> ExitCode {
    crash::install(CrashConfig { app: "runtime_info", version: env!("CARGO_PKG_VERSION"), dir: "crash" });
    global::set_budget(HEAP_BUDGET);
    let start_ticks = time::ticks();

    let Some(mut region) = MemoryRegion::reserve(REGION_SIZE) else {
        log::error(format_args!("sem memória: o SO recusou {REGION_SIZE_MB} MB"));
        return ExitCode::FAILURE;
    };
    let Some(mut memory) = region.thread_memory(&MAIN_THREAD_MEMORY) else {
        log::error(format_args!("a memória da thread principal não cabe em {REGION_SIZE_MB} MB"));
        return ExitCode::FAILURE;
    };
    let Some(dts) = memory.persistent.alloc_slice(FRAME_HISTORY, 0.0f32) else {
        memory.persistent.oom_fatal()
    };
    let mut history = FrameHistory { dts, next: 0, filled: 0 };

    let mut platform = match headless_frames_from_args() {
        Some(frames) => {
            log::info(format_args!("modo headless: {frames} frames"));
            Platform::headless(frames)
        }
        None => {
            let desc = WindowDesc { title: "Forja runtime info", width: 960, height: 540 };
            match Platform::init(&desc) {
                Ok(p) => p,
                Err(PlatformError::DisplayUnavailable) => {
                    log::error(format_args!("sem servidor gráfico (no Linux: X11/DISPLAY); rode com --headless N"));
                    return ExitCode::FAILURE;
                }
                Err(PlatformError::DisplayLibraryMissing) => {
                    log::error(format_args!(
                        "libX11.so.6 não encontrada (sudo apt install libx11-6); ou rode com --headless N"
                    ));
                    return ExitCode::FAILURE;
                }
                Err(e) => {
                    log::error(format_args!("falha ao criar janela: {e:?}"));
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    // Ainda na inicialização: a std aloca o buffer do stdout no primeiro uso e consulta os
    // núcleos; depois de `end_startup` isso apareceria no log como alocação fora de hora.
    let logical_cpus = std::thread::available_parallelism().map_or(0, |n| n.get());
    write_stdout(b"Forja runtime info | F1 relatorio | F2 aloca no heap geral | F9 crash | Esc sai\n");
    let startup =
        Startup { start_ticks, logical_cpus, main_thread_id: process::thread_id(), heap: global::stats() };
    global::end_startup();

    let mut heap_allocations_before = global::stats().allocations;
    let mut heap_allocations_last_frame = 0u64;
    let mut title_ticks = platform.frame_start;

    print_report(
        "inicialização",
        &ReportInput {
            startup: &startup,
            region: &region,
            platform: &platform,
            history: &history,
            heap_allocations_last_frame,
        },
        &memory.frame,
    );

    loop {
        crash::set_frame(platform.frame_index);
        if !platform.poll() || platform.input.key_pressed(Key::ESCAPE) {
            break;
        }
        history.add(platform.dt);

        if platform.input.key_pressed(KEY_REPORT) {
            print_report(
                "F1",
                &ReportInput {
                    startup: &startup,
                    region: &region,
                    platform: &platform,
                    history: &history,
                    heap_allocations_last_frame,
                },
                &memory.frame,
            );
        }
        if platform.input.key_pressed(KEY_HEAP_ALLOC) {
            heap_alloc_demo();
        }
        if platform.input.key_pressed(KEY_CRASH) {
            crash::fatal("F9 no runtime_info", format_args!("erro fatal de propósito, para ver o crash report"));
        }

        let since_title = platform.frame_start.saturating_sub(title_ticks);
        if time::ticks_to_seconds(since_title, platform.ticks_per_second) >= TITLE_INTERVAL_SECONDS {
            title_ticks = platform.frame_start;
            update_title(&platform, &history, heap_allocations_last_frame, &memory.frame);
        }

        if !platform.is_headless() {
            platform.wait_frame(TARGET_FRAME_SECONDS);
        }

        let heap_allocations = global::stats().allocations;
        heap_allocations_last_frame = heap_allocations.saturating_sub(heap_allocations_before);
        heap_allocations_before = heap_allocations;
        memory.frame.reset();
    }

    print_report(
        "final",
        &ReportInput {
            startup: &startup,
            region: &region,
            platform: &platform,
            history: &history,
            heap_allocations_last_frame,
        },
        &memory.frame,
    );
    platform.shutdown();
    ExitCode::SUCCESS
}

/// `--headless N` (N opcional). Lido na inicialização: a std aloca para os argumentos, o que é
/// permitido antes de `global::end_startup()`.
///
/// `args_os` em vez de `args`: `args` entra em pânico se algum argumento não for UTF-8 válido.
/// Aqui um argumento assim é só ignorado (e um N inválido vira o padrão).
fn headless_frames_from_args() -> Option<u64> {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--headless" {
            let frames = args.next().and_then(|n| n.to_str().and_then(|s| s.parse::<u64>().ok()));
            return Some(frames.unwrap_or(HEADLESS_DEFAULT_FRAMES));
        }
    }
    None
}

fn write_stdout(bytes: &[u8]) {
    let _ = std::io::Write::write_all(&mut std::io::stdout(), bytes);
}

/// F2: aloca e libera no heap geral, com a tag `tools`. Depois de `end_startup`, o heap geral
/// escreve no log o ponto de chamada com a pilha (uma vez por ponto); as próximas só contam.
fn heap_alloc_demo() {
    let previous = global::set_thread_tag(AllocTag::Tools);
    let block = std::hint::black_box(Vec::<u8>::with_capacity(HEAP_DEMO_BYTES));
    drop(block);
    global::set_thread_tag(previous);
    let tools = global::tag_stats(AllocTag::Tools);
    log::info(format_args!(
        "F2: {HEAP_DEMO_BYTES} bytes alocados e liberados no heap geral (tag tools: {} alocações)",
        tools.allocations
    ));
}

/// Fps, tempo de frame e heap no título. O texto vem da arena do frame.
fn update_title(platform: &Platform, history: &FrameHistory, heap_allocations_last_frame: u64, frame_arena: &Arena) {
    let s = history.summary();
    let Some(buf) = frame_arena.alloc_slice(TITLE_CAPACITY, 0u8) else {
        return;
    };
    let mut t = TextBuf::new(buf);
    let _ = write!(
        t,
        "Forja runtime info | {:.0} fps | {:.2} ms (pior {:.2}) | heap {} aloc/frame | F1 relatório",
        fps(s.average_s),
        s.average_s * 1000.0,
        s.worst_s * 1000.0,
        heap_allocations_last_frame
    );
    platform.set_title(t.as_str());
}

// ---------------------------------------------------------------- relatório

/// Monta o relatório num buffer da arena do frame e manda para o stdout.
fn print_report(heading: &str, r: &ReportInput, frame_arena: &Arena) {
    let Some(buf) = frame_arena.alloc_slice(REPORT_CAPACITY, 0u8) else {
        log::warn(format_args!("relatório: {REPORT_CAPACITY} bytes não cabem na arena do frame"));
        return;
    };
    let mut t = TextBuf::new(buf);
    let _ = writeln!(t, "\n=================== Forja runtime info: {heading} ===================");
    write_build(&mut t);
    write_os(&mut t, r.startup);
    write_machine_memory(&mut t);
    write_process_memory(&mut t);
    write_forja_memory(&mut t, r.region);
    write_heap(&mut t, r.startup, r.heap_allocations_last_frame);
    write_platform(&mut t, r.platform, r.history);
    write_input(&mut t, &r.platform.input, r.platform.is_headless());
    let _ = writeln!(t, "==================================================================");
    if t.truncated() {
        let _ = t.write_str("[relatório cortado: aumente REPORT_CAPACITY]\n");
    }
    write_stdout(t.as_bytes());
}

fn section(t: &mut TextBuf, name: &str) {
    let _ = writeln!(t, "{name}");
}

fn row(t: &mut TextBuf, label: &str, value: fmt::Arguments) {
    let _ = writeln!(t, "  {label:<26}{value}");
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / MB as f64
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / GB as f64
}

fn percent(part: f64, total: f64) -> f64 {
    if total <= 0.0 {
        return 0.0;
    }
    part * 100.0 / total
}

fn fps(average_s: f64) -> f64 {
    if average_s <= 0.0 {
        return 0.0;
    }
    1.0 / average_s
}

fn write_build(t: &mut TextBuf) {
    section(t, "build");
    row(t, "app", format_args!("runtime_info {}", env!("CARGO_PKG_VERSION")));
    row(t, "plataforma", format_args!("{} ({})", forja_os::CURRENT.name(), std::env::consts::ARCH));
    if cfg!(debug_assertions) {
        row(t, "perfil", format_args!("debug (heap geral: memória nova 0xCD, liberada 0xDD)"));
    } else {
        row(t, "perfil", format_args!("release"));
    }
}

fn write_os(t: &mut TextBuf, s: &Startup) {
    section(t, "sistema operacional");
    let pages = page_info();
    row(t, "página", format_args!("{} bytes", pages.page_size));
    row(t, "granularidade de reserva", format_args!("{} bytes", pages.allocation_granularity));
    let tps = time::ticks_per_second();
    row(t, "relógio monotônico", format_args!("{tps} ticks/s ({:.1} ns por tick)", 1e9 / tps as f64));
    let now = time::utc_now();
    row(
        t,
        "data e hora",
        format_args!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            now.year, now.month, now.day, now.hour, now.minute, now.second
        ),
    );
    let uptime = time::ticks_to_seconds(time::ticks().saturating_sub(s.start_ticks), tps);
    row(t, "tempo desde o início", format_args!("{uptime:.3} s"));
    row(t, "thread principal", format_args!("id {}", s.main_thread_id));
    if s.logical_cpus == 0 {
        row(t, "núcleos lógicos", format_args!("desconhecido"));
    } else {
        row(t, "núcleos lógicos", format_args!("{}", s.logical_cpus));
    }
}

fn write_machine_memory(t: &mut TextBuf) {
    section(t, "memória da máquina");
    let Some(m) = process::system_memory() else {
        row(t, "", format_args!("o SO não respondeu"));
        return;
    };
    row(t, "RAM total", format_args!("{:.1} GB", gb(m.total_physical)));
    row(
        t,
        "RAM disponível",
        format_args!(
            "{:.1} GB ({:.0}%)",
            gb(m.available_physical),
            percent(m.available_physical as f64, m.total_physical as f64)
        ),
    );
    match (m.commit_limit, m.commit_available) {
        (Some(limit), Some(available)) => {
            row(t, "commit (RAM + paginação)", format_args!("{:.1} GB livres de {:.1} GB", gb(available), gb(limit)));
        }
        _ => row(t, "commit (RAM + paginação)", format_args!("não existe neste SO")),
    }
}

fn write_process_memory(t: &mut TextBuf) {
    section(t, "memória do processo");
    let Some(p) = process::process_memory() else {
        row(t, "", format_args!("o SO não respondeu"));
        return;
    };
    row(t, "residente (RSS)", format_args!("{:.1} MB (pico {:.1} MB)", mb(p.resident), mb(p.peak_resident)));
    match (p.committed, p.peak_committed) {
        (Some(c), Some(peak)) => row(t, "commit", format_args!("{:.1} MB (pico {:.1} MB)", mb(c), mb(peak))),
        _ => row(t, "commit", format_args!("não existe neste SO")),
    }
}

fn write_forja_memory(t: &mut TextBuf, region: &MemoryRegion) {
    section(t, "memória da forja (uma reserva ao SO)");
    let capacity = region.capacity() as u64;
    let used = region.used() as u64;
    row(
        t,
        "região",
        format_args!(
            "{:.2} MB recortados de {:.2} MB ({:.0}%), página {} bytes",
            mb(used),
            mb(capacity),
            percent(used as f64, capacity as f64),
            region.page_size()
        ),
    );
    let count = stats::arena_count();
    row(t, "arenas", format_args!("{count} de {ARENAS_MAX}  (usado agora / pico / capacidade, em bytes)"));
    for i in 0..count {
        let Some(a) = stats::arena_snapshot(i) else {
            continue;
        };
        let _ = writeln!(
            t,
            "    {:<12} {:>9} / {:>9} / {:>9} ({:>3.0}% no pico), {} alocações, {} falhas",
            a.name,
            a.used,
            a.peak,
            a.capacity,
            percent(a.peak as f64, a.capacity as f64),
            a.allocations,
            a.failures
        );
    }
}

fn write_heap(t: &mut TextBuf, s: &Startup, heap_allocations_last_frame: u64) {
    section(t, "heap geral (std e dependências)");
    let h = global::stats();
    row(t, "política", format_args!("{:?}", h.policy));
    row(
        t,
        "bytes vivos",
        format_args!("{} (pico {}, orçamento {})", h.live_bytes, h.peak_bytes, h.budget),
    );
    row(t, "alocações / liberações", format_args!("{} / {}", h.allocations, h.frees));
    row(
        t,
        "na inicialização",
        format_args!("{} bytes vivos em {} alocações", s.heap.live_bytes, s.heap.allocations),
    );
    row(
        t,
        "depois da inicialização",
        format_args!("{} alocações", h.allocations.saturating_sub(s.heap.allocations)),
    );
    row(t, "no último frame", format_args!("{heap_allocations_last_frame} alocações (meta: 0)"));
    row(
        t,
        "pontos de chamada no log",
        format_args!("{} ({} sem espaço na tabela)", h.reported_sites, h.untracked_reports),
    );
    for tag in AllocTag::ALL {
        let ts = global::tag_stats(tag);
        if ts.allocations > 0 {
            let _ = writeln!(
                t,
                "    tag {:<12} {:>9} bytes vivos, {} alocações",
                tag.name(),
                ts.live_bytes,
                ts.allocations
            );
        }
    }
}

fn write_platform(t: &mut TextBuf, p: &Platform, history: &FrameHistory) {
    section(t, "plataforma");
    if p.is_headless() {
        row(t, "modo", format_args!("headless (sem janela)"));
    } else {
        row(t, "modo", format_args!("janela"));
        row(t, "área cliente", format_args!("{} x {} pixels", p.width, p.height));
        row(t, "foco", format_args!("{}", if p.focused { "sim" } else { "não" }));
    }
    row(t, "frame_index", format_args!("{} (o poll que encerra também conta)", p.frame_index));
    row(t, "dt do último frame", format_args!("{:.3} ms", f64::from(p.dt) * 1000.0));
    let s = history.summary();
    if s.frames == 0 {
        row(t, "tempo de frame", format_args!("ainda sem frames"));
    } else {
        row(
            t,
            "tempo de frame",
            format_args!(
                "média {:.3} ms, pior {:.3} ms, {:.1} fps (últimos {} frames)",
                s.average_s * 1000.0,
                s.worst_s * 1000.0,
                fps(s.average_s),
                s.frames
            ),
        );
    }
}

fn button_state(input: &Input, b: MouseButton) -> &'static str {
    if input.mouse_down(b) { "baixo" } else { "-" }
}

fn write_input(t: &mut TextBuf, input: &Input, headless: bool) {
    section(t, "input");
    if headless {
        row(t, "", format_args!("sem input no modo headless"));
        return;
    }
    row(t, "mouse", format_args!("({}, {})", input.mouse_x, input.mouse_y));
    row(
        t,
        "botões",
        format_args!(
            "esquerdo {}, direito {}, meio {}",
            button_state(input, MouseButton::Left),
            button_state(input, MouseButton::Right),
            button_state(input, MouseButton::Middle)
        ),
    );
    let _ = write!(t, "  {:<26}", "teclas baixas");
    let mut any = false;
    for code in 0..=u8::MAX {
        if !input.key_down(Key(code)) {
            continue;
        }
        any = true;
        if code.is_ascii_uppercase() || code.is_ascii_digit() {
            let _ = write!(t, "{} ", char::from(code));
        } else if (Key::F1.0..=Key::F12.0).contains(&code) {
            let _ = write!(t, "F{} ", code.wrapping_sub(Key::F1.0).wrapping_add(1));
        } else {
            let _ = write!(t, "0x{code:02X} ");
        }
    }
    if !any {
        let _ = t.write_str("nenhuma");
    }
    let _ = t.write_char('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_history_keeps_only_the_last_frames() {
        let mut dts = [0.0f32; 3];
        let mut h = FrameHistory { dts: &mut dts, next: 0, filled: 0 };
        assert_eq!(h.summary().frames, 0);
        h.add(0.010);
        h.add(0.030);
        assert_eq!(h.summary().frames, 2);
        assert!((h.summary().average_s - 0.020).abs() < 1e-6);
        // 4 frames num ring de 3: o primeiro (10 ms) sai.
        h.add(0.020);
        h.add(0.040);
        let s = h.summary();
        assert_eq!(s.frames, 3);
        assert!((s.average_s - 0.030).abs() < 1e-6);
        assert!((s.worst_s - 0.040).abs() < 1e-6);
    }
}
