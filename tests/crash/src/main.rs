//! Provoca um crash do tipo pedido e deixa o forja_diag gravar o relatório.
//!
//! crash_tests <mode> <dir>
//!
//! Modos: oom, panic, access, stack (todos crasham).

use std::hint::black_box;
use std::process::ExitCode;

use forja_diag::crash::{self, CrashConfig};
use forja_diag::log;
use forja_mem::region::MemoryRegion;

fn main() -> ExitCode {
    // `args_os` em vez de `args` (que entra em pânico com argumento que não é UTF-8): modo e
    // pasta precisam ser texto, então argumento inválido sai com erro, como modo desconhecido.
    let mut args = std::env::args_os().skip(1);
    let mode = match args.next().map(|m| m.into_string()) {
        None => String::new(),
        Some(Ok(m)) => m,
        Some(Err(m)) => {
            log::error(format_args!("modo não é UTF-8 válido: {m:?}"));
            return ExitCode::from(2);
        }
    };
    let dir: &'static str = match args.next().map(|d| d.into_string()) {
        None => "crash",
        Some(Ok(d)) => d.leak(),
        Some(Err(d)) => {
            log::error(format_args!("pasta não é UTF-8 válido: {d:?}"));
            return ExitCode::from(2);
        }
    };

    crash::install(CrashConfig { app: "crash_tests", version: env!("CARGO_PKG_VERSION"), dir });
    let Some(mut region) = MemoryRegion::reserve(1024 * 1024) else {
        log::error(format_args!("sem memória"));
        return ExitCode::from(2);
    };
    let Some(frame) = region.arena(4096) else {
        log::error(format_args!("arena não coube"));
        return ExitCode::from(2);
    };
    log::info(format_args!("crash_tests: modo '{mode}'"));
    crash::set_frame(42);

    match mode.as_str() {
        "oom" => {
            // Arena sem espaço é fatal quando o jogo decide: 3800 de 4096 cabem, 1000 não.
            let _ = frame.alloc_slice(3800, 0u8);
            if frame.alloc_slice(1000, 0u8).is_none() {
                crash::fatal(
                    "OOM de arena",
                    format_args!(
                        "arena 'frame' sem espaço: pedido de 1000 bytes com {} de {} usados",
                        frame.used(),
                        frame.capacity()
                    ),
                );
            }
        }
        "panic" => trigger_panic(black_box(7)),
        "access" => {
            // SAFETY: não é seguro: é o crash que queremos testar.
            unsafe { black_box(core::ptr::null_mut::<u32>()).write_volatile(1) };
        }
        "stack" => {
            black_box(recurse(0));
        }
        _ => {
            log::error(format_args!("modo desconhecido: '{mode}'"));
            return ExitCode::from(2);
        }
    }
    log::error(format_args!("modo '{mode}' não crashou"));
    ExitCode::from(3)
}

#[allow(clippy::panic)]
fn trigger_panic(i: usize) {
    panic!("teste de panic: índice {i}");
}

#[allow(unconditional_recursion)]
fn recurse(n: u64) -> u64 {
    let frame = black_box([n as u8; 4096]);
    let r = recurse(black_box(n.wrapping_add(1)));
    r.wrapping_add(u64::from(frame.first().copied().unwrap_or(0)))
}
