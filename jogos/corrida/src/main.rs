//! Corrida — M0: janela, loop, input e profiler.
//! Ainda sem render: velocidade e tempo de frame aparecem no título da janela.
//!
//! Controles: W acelera, S freia, Esc sai.
//! Ao sair, grava `corrida.trace.json` (abrir em https://ui.perfetto.dev).

use std::process::ExitCode;

use forja_core::log::{log_erro, log_info};
use forja_core::math::f32_clamp;
use forja_inspect::{
    ATIVO, Profiler, prof_exportar_chrome_trace, prof_fim, prof_inicio, prof_tsc_por_segundo,
    profiler_create,
};
use forja_platform::input::{TECLA_ESC, input_tecla_baixo, input_tecla_pressionou};
use forja_platform::{
    JanelaDesc, Platform, platform_esperar_frame, platform_init, platform_poll, platform_shutdown,
    platform_titulo,
};

// ---------------------------------------------------------------- zonas do profiler

const ZONA_FRAME: u16 = 0;
const ZONA_PLATAFORMA: u16 = 1;
const ZONA_UPDATE: u16 = 2;
const ZONA_ESPERA: u16 = 3;
static NOMES_ZONAS: [&str; 4] = ["frame", "plataforma", "update", "espera"];

/// ~1 hora a 60 fps com 8 eventos por frame (16 MB).
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

/// A cada meio segundo, mostra média e pior frame no título.
fn estatistica_titulo(e: &mut Estatistica, plat: &Platform, carro: &Carro) {
    if e.acumulado_s < 0.5 || e.frames == 0 {
        return;
    }
    let media_ms = e.acumulado_s / f64::from(e.frames) * 1000.0;
    let pior_ms = e.pior_s * 1000.0;
    let kmh = carro.velocidade * 3.6;
    platform_titulo(
        plat,
        &format!("Forja corrida | {media_ms:.2} ms (pior {pior_ms:.2}) | {kmh:.0} km/h"),
    );
    *e = Estatistica { acumulado_s: 0.0, pior_s: 0.0, frames: 0 };
}

// ---------------------------------------------------------------- main

fn main() -> ExitCode {
    let desc = JanelaDesc { titulo: "Forja corrida", largura: 1280, altura: 720 };
    let mut plat = match platform_init(&desc) {
        Ok(p) => p,
        Err(e) => {
            log_erro(&format!("falha ao criar janela: {e:?}"));
            return ExitCode::FAILURE;
        }
    };
    let mut prof = profiler_create(&NOMES_ZONAS, PROF_CAPACIDADE);
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
        estatistica_titulo(&mut est, &plat, &carro);
        prof_fim(&mut prof, ZONA_UPDATE);

        prof_inicio(&mut prof, ZONA_ESPERA);
        platform_esperar_frame(&plat, FRAME_ALVO_S);
        prof_fim(&mut prof, ZONA_ESPERA);

        prof_fim(&mut prof, ZONA_FRAME);
    }

    platform_shutdown(plat);
    salvar_trace(&prof);
    ExitCode::SUCCESS
}

fn salvar_trace(prof: &Profiler) {
    if !ATIVO {
        return;
    }
    let mut json = String::with_capacity(prof.eventos.len().saturating_mul(64));
    prof_exportar_chrome_trace(prof, prof_tsc_por_segundo(prof), &mut json);
    match std::fs::write(TRACE_ARQUIVO, json.as_bytes()) {
        Ok(()) => log_info(&format!(
            "trace salvo em {TRACE_ARQUIVO} ({} eventos, {} perdidos)",
            prof.eventos.len(),
            prof.perdidos
        )),
        Err(e) => log_erro(&format!("falha ao salvar {TRACE_ARQUIVO}: {e}")),
    }
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
