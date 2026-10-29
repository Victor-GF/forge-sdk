//! Corrida — M0: janela, loop, input, profiler e memória sem malloc.
//! Ainda sem render: velocidade e tempo de frame aparecem no título da janela.
//!
//! Controles: W acelera, S freia, Esc sai.
//! Ao sair, grava `corrida.trace.json` (abrir em https://ui.perfetto.dev).
//!
//! Memória: um único `VirtualAlloc` com o orçamento abaixo; a thread principal trabalha na
//! sua mesa (persistente, fase, frame). Depois da inicialização o alocador global é travado:
//! qualquer `Vec`/`String`/`Box` aborta o programa.

use core::fmt::Write;
use std::process::ExitCode;

use forja_core::log::{log_erro, log_info};
use forja_core::math::f32_clamp;
use forja_core::texto::{texto_novo, texto_str};
use forja_inspect::{
    ATIVO, PROF_EVENTO_VAZIO, Profiler, prof_contador, prof_exportar_bloco, prof_exportar_inicio,
    prof_fim, prof_inicio, prof_tsc_por_segundo, profiler_create,
};
use forja_mem::arena::{Arena, arena_estatistica, arena_reset, arena_slice, arena_usado};
use forja_mem::global::{global_estatistica, global_travar};
use forja_mem::memoria::{Mesa, MesaDesc, memoria_init, memoria_mesa};
use forja_platform::arquivo::{arquivo_criar, arquivo_escrever, arquivo_fechar};
use forja_platform::input::{TECLA_ESC, input_tecla_baixo, input_tecla_pressionou};
use forja_platform::{
    JanelaDesc, Platform, platform_esperar_frame, platform_init, platform_poll, platform_shutdown,
    platform_titulo,
};

#[cfg(not(test))]
#[global_allocator]
static ALOCADOR: forja_mem::global::AlocadorGlobal = forja_mem::global::AlocadorGlobal;

// ---------------------------------------------------------------- orçamento de memória

const MB: usize = 1024 * 1024;
const MESA_PRINCIPAL: MesaDesc = MesaDesc { persistente: 32 * MB, fase: 16 * MB, frame: 4 * MB };
/// Uma mesa por enquanto; sobra para mesas de outras threads no futuro.
const MEMORIA_TOTAL: usize = 64 * MB;
const MEMORIA_TOTAL_MB: usize = MEMORIA_TOTAL / MB;

// ---------------------------------------------------------------- profiler

const ZONA_FRAME: u16 = 0;
const ZONA_PLATAFORMA: u16 = 1;
const ZONA_UPDATE: u16 = 2;
const ZONA_ESPERA: u16 = 3;
const CONT_MEM_FRAME: u16 = 4;
const CONT_MEM_GLOBAL: u16 = 5;
static NOMES_PROF: [&str; 6] = [
    "frame",
    "plataforma",
    "update",
    "espera",
    "mem frame (bytes)",
    "mem global (bytes)",
];

/// ~30 min a 60 fps com 10 eventos por frame (16 MB da arena persistente).
const PROF_CAPACIDADE: usize = 1 << 20;
const TRACE_ARQUIVO: &str = "corrida.trace.json";

const FRAME_ALVO_S: f64 = 1.0 / 60.0;
/// Limite do dt da simulação (pausa no debugger, janela arrastada).
const DT_MAX: f32 = 0.1;

// ---------------------------------------------------------------- carro (provisório)

const CARRO_ACEL: f32 = 12.0; // m/s²
const CARRO_FREIO: f32 = 25.0; // m/s²
const CARRO_ARRASTO: f32 = 2.0; // m/s² sem acelerar
const CARRO_VEL_MAX: f32 = 83.0; // m/s (~300 km/h)

#[repr(C)]
struct Carro {
    velocidade: f32, // m/s
}

fn carro_update(c: &mut Carro, acelera: bool, freia: bool, dt: f32) {
    let a = if freia {
        -CARRO_FREIO
    } else if acelera {
        CARRO_ACEL
    } else {
        -CARRO_ARRASTO
    };
    c.velocidade = f32_clamp(c.velocidade + a * dt, 0.0, CARRO_VEL_MAX);
}

// ---------------------------------------------------------------- estatística do título

struct Estatistica {
    acumulado_s: f64,
    pior_s: f64,
    frames: u32,
}

fn estatistica_frame(e: &mut Estatistica, dt: f32) {
    let dt = f64::from(dt);
    e.acumulado_s += dt;
    if dt > e.pior_s {
        e.pior_s = dt;
    }
    e.frames = e.frames.saturating_add(1);
}

/// A cada meio segundo, mostra média e pior frame no título. O texto vem da arena do frame.
fn estatistica_titulo(e: &mut Estatistica, plat: &Platform, carro: &Carro, frame: &Arena) {
    if e.acumulado_s < 0.5 || e.frames == 0 {
        return;
    }
    let media_ms = e.acumulado_s / f64::from(e.frames) * 1000.0;
    let pior_ms = e.pior_s * 1000.0;
    let kmh = carro.velocidade * 3.6;
    *e = Estatistica { acumulado_s: 0.0, pior_s: 0.0, frames: 0 };

    let Some(buf) = arena_slice(frame, 128, 0u8) else {
        return;
    };
    let mut t = texto_novo(buf);
    let _ = write!(t, "Forja corrida | {media_ms:.2} ms (pior {pior_ms:.2}) | {kmh:.0} km/h");
    platform_titulo(plat, texto_str(&t));
}

// ---------------------------------------------------------------- main

fn main() -> ExitCode {
    let Some(mut memoria) = memoria_init(MEMORIA_TOTAL) else {
        log_erro(format_args!("sem memória: o SO recusou {MEMORIA_TOTAL_MB} MB"));
        return ExitCode::FAILURE;
    };
    let Some(mut mesa) = memoria_mesa(&mut memoria, &MESA_PRINCIPAL) else {
        log_erro(format_args!("a mesa principal não cabe em {MEMORIA_TOTAL_MB} MB"));
        return ExitCode::FAILURE;
    };

    let prof_cap = if ATIVO { PROF_CAPACIDADE } else { 0 };
    let Some(eventos) = arena_slice(&mesa.persistente, prof_cap, PROF_EVENTO_VAZIO) else {
        log_erro(format_args!("buffer do profiler não cabe na arena persistente"));
        return ExitCode::FAILURE;
    };
    let mut prof = profiler_create(&NOMES_PROF, eventos);

    let desc = JanelaDesc { titulo: "Forja corrida", largura: 1280, altura: 720 };
    let mut plat = match platform_init(&desc) {
        Ok(p) => p,
        Err(e) => {
            log_erro(format_args!("falha ao criar janela: {e:?}"));
            return ExitCode::FAILURE;
        }
    };

    let g = global_estatistica();
    log_info(format_args!(
        "inicialização: {} bytes em {} alocações globais (std); travando o alocador global",
        g.usado, g.alocacoes
    ));
    global_travar();

    let mut carro = Carro { velocidade: 0.0 };
    let mut est = Estatistica { acumulado_s: 0.0, pior_s: 0.0, frames: 0 };

    loop {
        prof_inicio(&mut prof, ZONA_FRAME);

        prof_inicio(&mut prof, ZONA_PLATAFORMA);
        let rodando = platform_poll(&mut plat);
        prof_fim(&mut prof, ZONA_PLATAFORMA);
        if !rodando || input_tecla_pressionou(&plat.input, TECLA_ESC) {
            prof_fim(&mut prof, ZONA_FRAME);
            break;
        }

        prof_inicio(&mut prof, ZONA_UPDATE);
        let dt = plat.dt.min(DT_MAX);
        let acelera = input_tecla_baixo(&plat.input, b'W');
        let freia = input_tecla_baixo(&plat.input, b'S');
        carro_update(&mut carro, acelera, freia, dt);
        estatistica_frame(&mut est, plat.dt);
        estatistica_titulo(&mut est, &plat, &carro, &mesa.frame);
        prof_fim(&mut prof, ZONA_UPDATE);

        prof_inicio(&mut prof, ZONA_ESPERA);
        platform_esperar_frame(&plat, FRAME_ALVO_S);
        prof_fim(&mut prof, ZONA_ESPERA);

        fim_de_frame(&mut prof, &mut mesa.frame);
        prof_fim(&mut prof, ZONA_FRAME);
    }

    platform_shutdown(plat);
    salvar_trace(&prof);
    log_memoria(&mesa);
    ExitCode::SUCCESS
}

/// Registra o uso de memória do frame e zera a arena do frame.
fn fim_de_frame(prof: &mut Profiler, frame: &mut Arena) {
    let usado = u32::try_from(arena_usado(frame)).unwrap_or(u32::MAX);
    prof_contador(prof, CONT_MEM_FRAME, usado);
    let global = u32::try_from(global_estatistica().usado).unwrap_or(u32::MAX);
    prof_contador(prof, CONT_MEM_GLOBAL, global);
    arena_reset(frame);
}

/// Exporta em blocos de 64 KB (buffer na stack) direto para o arquivo.
fn salvar_trace(prof: &Profiler) {
    if !ATIVO {
        return;
    }
    let Some(arq) = arquivo_criar(TRACE_ARQUIVO) else {
        log_erro(format_args!("falha ao criar {TRACE_ARQUIVO}"));
        return;
    };
    let mut exp = prof_exportar_inicio(prof_tsc_por_segundo(prof));
    let mut buf = [0u8; 64 * 1024];
    let mut ok = true;
    loop {
        let n = prof_exportar_bloco(prof, &mut exp, &mut buf);
        let Some(pedaco) = buf.get(..n) else {
            break;
        };
        if n == 0 {
            break;
        }
        if !arquivo_escrever(&arq, pedaco) {
            ok = false;
            break;
        }
    }
    arquivo_fechar(arq);
    if ok {
        log_info(format_args!(
            "trace salvo em {TRACE_ARQUIVO} ({} eventos, {} perdidos)",
            prof.usados, prof.perdidos
        ));
    } else {
        log_erro(format_args!("falha ao gravar {TRACE_ARQUIVO}"));
    }
}

fn log_memoria(mesa: &Mesa) {
    for a in [&mesa.persistente, &mesa.fase, &mesa.frame] {
        let e = arena_estatistica(a);
        log_info(format_args!(
            "arena {:<11} pico {:>9} / {:>9} bytes, {} alocações, {} falhas",
            a.nome, e.pico, e.capacidade, e.alocacoes, e.falhas
        ));
    }
    let g = global_estatistica();
    log_info(format_args!(
        "global: {} / {} bytes, {} alocações, {} liberações (travado: {})",
        g.usado, g.capacidade, g.alocacoes, g.liberacoes, g.travado
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carro_acelera_freia_e_respeita_limites() {
        let mut c = Carro { velocidade: 0.0 };
        carro_update(&mut c, true, false, 1.0);
        assert_eq!(c.velocidade, CARRO_ACEL);
        carro_update(&mut c, true, true, 1.0);
        assert_eq!(c.velocidade, 0.0);
        for _ in 0..100 {
            carro_update(&mut c, true, false, 1.0);
        }
        assert_eq!(c.velocidade, CARRO_VEL_MAX);
    }
}
