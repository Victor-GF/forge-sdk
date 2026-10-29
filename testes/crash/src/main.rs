//! Provoca um crash do tipo pedido e deixa o forja_diag gravar o relatório.
//!
//! crash_testes <modo> <pasta>
//!
//! Modos: oom, travado, esgotado, panic, acesso, pilha.

use std::hint::black_box;
use std::process::ExitCode;

use forja_diag::crash::{DiagDesc, diag_frame, diag_instalar};
use forja_diag::log::{log_erro, log_info};
use forja_mem::arena::{arena_oom_fatal, arena_slice};
use forja_mem::global::{AlocadorGlobal, GLOBAL_CAPACIDADE, global_travar};
use forja_mem::memoria::{MesaDesc, memoria_init, memoria_mesa};

#[global_allocator]
static ALOCADOR: AlocadorGlobal = AlocadorGlobal;

fn main() -> ExitCode {
    // Argumentos lidos antes de travar o alocador (a std aloca para eles).
    let mut args = std::env::args();
    let _ = args.next();
    let modo = args.next().unwrap_or_default();
    let pasta: &'static str = args.next().unwrap_or_else(|| String::from("crash")).leak();

    diag_instalar(DiagDesc { jogo: "crash_testes", versao: env!("CARGO_PKG_VERSION"), pasta });
    let Some(mut memoria) = memoria_init(1024 * 1024) else {
        log_erro(format_args!("sem memória"));
        return ExitCode::from(2);
    };
    let desc = MesaDesc { persistente: 4096, fase: 4096, frame: 4096 };
    let Some(mesa) = memoria_mesa(&mut memoria, &desc) else {
        log_erro(format_args!("mesa não coube"));
        return ExitCode::from(2);
    };
    log_info(format_args!("crash_testes: modo '{modo}'"));
    diag_frame(42);

    if modo == "esgotado" {
        // Antes de travar: pede mais que o buffer do alocador global.
        let v: Vec<u8> = Vec::with_capacity(black_box(GLOBAL_CAPACIDADE.saturating_mul(2)));
        black_box(&v);
    }

    global_travar();

    match modo.as_str() {
        "oom" => {
            // 3800 de 4096 passa de 90% (aviso no log); os próximos 1000 não cabem.
            let _ = arena_slice(&mesa.frame, 3800, 0u8);
            let Some(_) = arena_slice(&mesa.frame, 1000, 0u8) else {
                arena_oom_fatal(&mesa.frame)
            };
        }
        "travado" => {
            black_box(Vec::<u8>::with_capacity(black_box(100)));
        }
        "panic" => provocar_panic(black_box(7)),
        "acesso" => {
            // SAFETY: não é seguro: é o crash que queremos testar.
            unsafe { black_box(core::ptr::null_mut::<u32>()).write_volatile(1) };
        }
        "pilha" => {
            black_box(recursao(0));
        }
        _ => {
            log_erro(format_args!("modo desconhecido: '{modo}'"));
            return ExitCode::from(2);
        }
    }
    log_erro(format_args!("modo '{modo}' não crashou"));
    ExitCode::from(3)
}

#[allow(clippy::panic)]
fn provocar_panic(i: usize) {
    panic!("teste de panic: índice {i}");
}

#[allow(unconditional_recursion)]
fn recursao(n: u64) -> u64 {
    let quadro = black_box([n as u8; 4096]);
    let r = recursao(black_box(n.wrapping_add(1)));
    r.wrapping_add(u64::from(quadro.first().copied().unwrap_or(0)))
}
