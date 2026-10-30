//! Provoca um crash do tipo pedido e deixa o forja_diag gravar o relatório.
//!
//! crash_tests <mode> <dir>
//!
//! Modos que crasham: oom, forbid, heap_oom, panic, access, stack.
//! Modo que não crasha: report (aloca no heap geral depois da inicialização e sai com 0).

use std::hint::black_box;
use std::process::ExitCode;

use forja_diag::crash::{self, CrashConfig};
use forja_diag::log;
use forja_mem::global::{self, GlobalAllocator, HeapPolicy};
use forja_mem::region::{MemoryRegion, ThreadMemoryDesc};

#[global_allocator]
static ALLOCATOR: GlobalAllocator = GlobalAllocator;

fn main() -> ExitCode {
    // Inicialização: a std aloca para os argumentos (política Startup, sem relatório).
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
    let desc = ThreadMemoryDesc { persistent: 4096, level: 4096, frame: 4096 };
    let Some(memory) = region.thread_memory(&desc) else {
        log::error(format_args!("memória da thread não coube"));
        return ExitCode::from(2);
    };
    log::info(format_args!("crash_tests: modo '{mode}'"));
    crash::set_frame(42);

    if mode == "heap_oom" {
        // 64 TB: o heap do sistema recusa.
        black_box(Vec::<u8>::with_capacity(black_box(1usize << 46)));
    }

    global::end_startup();

    match mode.as_str() {
        "oom" => {
            // 3800 de 4096 passa de 90% (aviso no log); os próximos 1000 não cabem.
            let _ = memory.frame.alloc_slice(3800, 0u8);
            let Some(_) = memory.frame.alloc_slice(1000, 0u8) else {
                memory.frame.oom_fatal()
            };
        }
        "forbid" => {
            global::set_policy(HeapPolicy::Forbid);
            black_box(Vec::<u8>::with_capacity(black_box(100)));
        }
        "report" => {
            global::set_budget(1000);
            // Mesmo ponto de chamada 3 vezes: um relatório só. (`black_box` na contagem impede
            // o compilador de desenrolar o loop em 3 chamadas, que seriam 3 pontos distintos.)
            for _ in 0..black_box(3) {
                allocate_small();
            }
            // Outro ponto de chamada, acima do orçamento: segundo relatório e aviso de orçamento.
            black_box(Vec::<u8>::with_capacity(black_box(2000)));
            let h = global::stats();
            log::info(format_args!("report: {} pontos de chamada relatados", h.reported_sites));
            return ExitCode::SUCCESS;
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

#[inline(never)]
fn allocate_small() {
    black_box(Vec::<u8>::with_capacity(black_box(100)));
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
