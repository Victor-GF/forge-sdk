//! Profiler por zonas e contadores. Grava eventos num slice fornecido por quem chama
//! (normalmente da arena persistente) e exporta no formato Chrome Trace (JSON), que abre em
//! https://ui.perfetto.dev. Não aloca.
//!
//! - Zonas e contadores são índices `u16` numa tabela de nomes que o jogo define.
//! - Relógio: `rdtsc` (poucos ns por evento), calibrado contra `Instant` na exportação.
//! - Uma thread só por enquanto (todos os eventos vão para `tid` 1).
//! - Buffer cheio: eventos novos são descartados e contados em `perdidos`.
//!
//! Sem a feature `ativo`, `ATIVO` é `false` e as funções de registro não fazem nada.

use core::fmt::Write;
use std::time::Instant;

use forja_core::texto::{Texto, texto_bytes, texto_novo};

pub const ATIVO: bool = cfg!(feature = "ativo");

pub const EVENTO_INICIO: u16 = 0;
pub const EVENTO_FIM: u16 = 1;
pub const EVENTO_CONTADOR: u16 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfEvento {
    pub tsc: u64,
    pub zona: u16,
    pub tipo: u16,
    /// Só para `EVENTO_CONTADOR`.
    pub valor: u32,
}

pub const PROF_EVENTO_VAZIO: ProfEvento = ProfEvento { tsc: 0, zona: 0, tipo: 0, valor: 0 };

pub struct Profiler<'a> {
    pub nomes: &'static [&'static str],
    pub eventos: &'a mut [ProfEvento],
    pub usados: usize,
    pub perdidos: u64,
    pub base_instante: Instant,
    pub base_tsc: u64,
}

#[cfg(target_arch = "x86_64")]
fn prof_relogio() -> u64 {
    // SAFETY: rdtsc só lê o contador da CPU; todo x86_64 suporta.
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(target_arch = "x86_64"))]
fn prof_relogio() -> u64 {
    match std::time::UNIX_EPOCH.elapsed() {
        Ok(d) => u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

/// `eventos` é o buffer (16 bytes por evento). Com `ATIVO == false`, passe um slice vazio.
pub fn profiler_create<'a>(nomes: &'static [&'static str], eventos: &'a mut [ProfEvento]) -> Profiler<'a> {
    Profiler {
        nomes,
        eventos,
        usados: 0,
        perdidos: 0,
        base_instante: Instant::now(),
        base_tsc: prof_relogio(),
    }
}

fn prof_registrar(p: &mut Profiler, zona: u16, tipo: u16, valor: u32) {
    if !ATIVO {
        return;
    }
    let Some(e) = p.eventos.get_mut(p.usados) else {
        p.perdidos = p.perdidos.saturating_add(1);
        return;
    };
    *e = ProfEvento { tsc: prof_relogio(), zona, tipo, valor };
    p.usados = p.usados.saturating_add(1);
}

pub fn prof_inicio(p: &mut Profiler, zona: u16) {
    prof_registrar(p, zona, EVENTO_INICIO, 0);
}

pub fn prof_fim(p: &mut Profiler, zona: u16) {
    prof_registrar(p, zona, EVENTO_FIM, 0);
}

/// Valor de um contador neste instante (ex.: bytes usados na arena do frame).
pub fn prof_contador(p: &mut Profiler, zona: u16, valor: u32) {
    prof_registrar(p, zona, EVENTO_CONTADOR, valor);
}

/// Frequência do relógio medida desde `profiler_create`. Quanto mais tempo rodou,
/// mais precisa.
pub fn prof_tsc_por_segundo(p: &Profiler) -> f64 {
    let segundos = p.base_instante.elapsed().as_secs_f64();
    let ticks = prof_relogio().saturating_sub(p.base_tsc);
    if segundos <= 0.0 {
        return 1.0;
    }
    ticks as f64 / segundos
}

// ---------------------------------------------------------------- exportação

const ETAPA_CABECALHO: u8 = 0;
const ETAPA_EVENTOS: u8 = 1;
const ETAPA_RODAPE: u8 = 2;
const ETAPA_FIM: u8 = 3;

/// Maior linha JSON gerada (um evento). Nomes que estourarem saem cortados.
const LINHA_MAX: usize = 512;

/// Estado da exportação em blocos; crie com `prof_exportar_inicio`.
pub struct ProfExportacao {
    pub etapa: u8,
    pub proximo: usize,
    pub micro_por_tick: f64,
}

pub fn prof_exportar_inicio(tsc_por_segundo: f64) -> ProfExportacao {
    let micro_por_tick = if tsc_por_segundo > 0.0 { 1.0e6 / tsc_por_segundo } else { 0.0 };
    ProfExportacao { etapa: ETAPA_CABECALHO, proximo: 0, micro_por_tick }
}

/// Preenche `saida` com o próximo pedaço do JSON Chrome Trace e devolve quantos bytes
/// escreveu. `0` significa que terminou. Chame em loop, gravando cada pedaço.
pub fn prof_exportar_bloco(p: &Profiler, exp: &mut ProfExportacao, saida: &mut [u8]) -> usize {
    let mut escrito: usize = 0;
    while exp.etapa != ETAPA_FIM {
        if exp.etapa == ETAPA_EVENTOS && exp.proximo >= p.usados {
            exp.etapa = ETAPA_RODAPE;
        }
        let mut linha_buf = [0u8; LINHA_MAX];
        let mut linha = texto_novo(&mut linha_buf);
        prof_linha(p, exp, &mut linha);
        let bytes = texto_bytes(&linha);
        let fim = escrito.saturating_add(bytes.len());
        match saida.get_mut(escrito..fim) {
            Some(destino) => {
                destino.copy_from_slice(bytes);
                escrito = fim;
            }
            // Não coube: fica para o próximo bloco.
            None if escrito > 0 => return escrito,
            // Maior que `saida` inteira: descarta para não travar.
            None => {}
        }
        prof_avancar(exp);
    }
    escrito
}

fn prof_avancar(exp: &mut ProfExportacao) {
    match exp.etapa {
        ETAPA_CABECALHO => exp.etapa = ETAPA_EVENTOS,
        ETAPA_EVENTOS => exp.proximo = exp.proximo.saturating_add(1),
        _ => exp.etapa = ETAPA_FIM,
    }
}

// `write!` em `Texto` nunca falha (corta o excesso); os resultados são ignorados.
fn prof_linha(p: &Profiler, exp: &ProfExportacao, linha: &mut Texto) {
    match exp.etapa {
        ETAPA_CABECALHO => {
            let _ = linha.write_str(
                "{\"traceEvents\":[\n\
                 {\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":1,\"args\":{\"name\":\"principal\"}}",
            );
        }
        ETAPA_EVENTOS => {
            let Some(e) = p.eventos.get(exp.proximo) else {
                return;
            };
            let nome = p.nomes.get(usize::from(e.zona)).copied().unwrap_or("?");
            let ts = e.tsc.saturating_sub(p.base_tsc) as f64 * exp.micro_por_tick;
            let _ = linha.write_str(",\n{\"name\":");
            json_texto(linha, nome);
            match e.tipo {
                EVENTO_CONTADOR => {
                    let _ = write!(
                        linha,
                        ",\"ph\":\"C\",\"ts\":{ts:.3},\"pid\":1,\"tid\":1,\"args\":{{\"valor\":{}}}}}",
                        e.valor
                    );
                }
                tipo => {
                    let fase = if tipo == EVENTO_INICIO { "B" } else { "E" };
                    let _ = write!(linha, ",\"ph\":\"{fase}\",\"ts\":{ts:.3},\"pid\":1,\"tid\":1}}");
                }
            }
        }
        ETAPA_RODAPE => {
            let _ = write!(
                linha,
                "\n],\"displayTimeUnit\":\"ms\",\"otherData\":{{\"eventos_perdidos\":{}}}}}\n",
                p.perdidos
            );
        }
        _ => {}
    }
}

fn json_texto(saida: &mut Texto, s: &str) {
    let _ = saida.write_char('"');
    for c in s.chars() {
        let _ = match c {
            '"' => saida.write_str("\\\""),
            '\\' => saida.write_str("\\\\"),
            c if u32::from(c) < 0x20 => write!(saida, "\\u{:04x}", u32::from(c)),
            c => saida.write_char(c),
        };
    }
    let _ = saida.write_char('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONA_A: u16 = 0;
    const ZONA_B: u16 = 1;
    const CONT_C: u16 = 2;
    static NOMES: [&str; 3] = ["frame", "com \"aspas\"", "mem"];

    fn exportar(p: &Profiler, bloco: usize) -> String {
        let mut exp = prof_exportar_inicio(1.0e9);
        let mut buf = vec![0u8; bloco];
        let mut tudo = Vec::new();
        loop {
            let n = prof_exportar_bloco(p, &mut exp, &mut buf);
            if n == 0 {
                break;
            }
            tudo.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(tudo).unwrap()
    }

    #[test]
    fn exporta_zonas_e_contadores_em_blocos() {
        let mut ev = [PROF_EVENTO_VAZIO; 16];
        let mut p = profiler_create(&NOMES, &mut ev);
        prof_inicio(&mut p, ZONA_A);
        prof_inicio(&mut p, ZONA_B);
        prof_fim(&mut p, ZONA_B);
        prof_contador(&mut p, CONT_C, 4096);
        prof_fim(&mut p, ZONA_A);

        // bloco pequeno força várias chamadas; o resultado deve ser idêntico ao bloco grande
        let s = exportar(&p, 200);
        assert_eq!(s, exportar(&p, 1 << 16));
        if !ATIVO {
            assert_eq!(p.usados, 0);
            assert!(!s.contains("\"ph\":\"B\""));
            return;
        }
        assert_eq!(p.usados, 5);
        assert_eq!(s.matches("\"ph\":\"B\"").count(), 2);
        assert_eq!(s.matches("\"ph\":\"E\"").count(), 2);
        assert!(s.contains("\"name\":\"mem\",\"ph\":\"C\""));
        assert!(s.contains("\"args\":{\"valor\":4096}"));
        assert!(s.contains("\"name\":\"com \\\"aspas\\\"\""));
        assert!(s.ends_with("\"eventos_perdidos\":0}}\n"));
    }

    #[test]
    fn bloco_menor_que_uma_linha_termina() {
        let mut ev = [PROF_EVENTO_VAZIO; 4];
        let mut p = profiler_create(&NOMES, &mut ev);
        prof_inicio(&mut p, ZONA_A);
        let mut exp = prof_exportar_inicio(1.0e9);
        let mut buf = [0u8; 8];
        let mut chamadas = 0u32;
        while prof_exportar_bloco(&p, &mut exp, &mut buf) != 0 {
            chamadas = chamadas.saturating_add(1);
            assert!(chamadas < 10);
        }
    }

    #[test]
    fn buffer_cheio_descarta_e_conta() {
        let mut ev = [PROF_EVENTO_VAZIO; 2];
        let mut p = profiler_create(&NOMES, &mut ev);
        prof_inicio(&mut p, ZONA_A);
        prof_fim(&mut p, ZONA_A);
        prof_inicio(&mut p, ZONA_A);
        if ATIVO {
            assert_eq!(p.usados, 2);
            assert_eq!(p.perdidos, 1);
        } else {
            assert_eq!(p.perdidos, 0);
        }
    }
}
