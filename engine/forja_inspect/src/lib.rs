//! Profiler por zonas. Grava eventos de início/fim num buffer de capacidade fixa e
//! exporta no formato Chrome Trace (JSON), que abre em https://ui.perfetto.dev.
//!
//! - Zonas são índices `u16` numa tabela de nomes que o jogo define (`&'static [&'static str]`).
//! - Relógio: `rdtsc` (poucos ns por evento), calibrado contra `Instant` na exportação.
//! - Uma thread só por enquanto (todos os eventos vão para `tid` 1).
//! - Buffer cheio: eventos novos são descartados e contados em `perdidos`.
//!
//! Sem a feature `ativo`, `ATIVO` é `false` e as funções de registro não fazem nada.

use core::fmt::Write;
use std::time::Instant;

pub const ATIVO: bool = cfg!(feature = "ativo");

pub const EVENTO_INICIO: u16 = 0;
pub const EVENTO_FIM: u16 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfEvento {
    pub tsc: u64,
    pub zona: u16,
    pub tipo: u16,
}

pub struct Profiler {
    pub nomes: &'static [&'static str],
    /// Capacidade fixa reservada na criação; nunca cresce.
    pub eventos: Vec<ProfEvento>,
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

/// `capacidade` em eventos (16 bytes cada). Com `ATIVO == false` nada é reservado.
pub fn profiler_create(nomes: &'static [&'static str], capacidade: usize) -> Profiler {
    let cap = if ATIVO { capacidade } else { 0 };
    Profiler {
        nomes,
        eventos: Vec::with_capacity(cap),
        perdidos: 0,
        base_instante: Instant::now(),
        base_tsc: prof_relogio(),
    }
}

fn prof_registrar(p: &mut Profiler, zona: u16, tipo: u16) {
    if !ATIVO {
        return;
    }
    if p.eventos.len() >= p.eventos.capacity() {
        p.perdidos = p.perdidos.saturating_add(1);
        return;
    }
    p.eventos.push(ProfEvento { tsc: prof_relogio(), zona, tipo });
}

pub fn prof_inicio(p: &mut Profiler, zona: u16) {
    prof_registrar(p, zona, EVENTO_INICIO);
}

pub fn prof_fim(p: &mut Profiler, zona: u16) {
    prof_registrar(p, zona, EVENTO_FIM);
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

/// Escreve o JSON Chrome Trace em `saida` (acrescenta; não limpa antes).
pub fn prof_exportar_chrome_trace(p: &Profiler, tsc_por_segundo: f64, saida: &mut String) {
    let micro_por_tick = if tsc_por_segundo > 0.0 { 1.0e6 / tsc_por_segundo } else { 0.0 };

    // `write!` em String nunca falha; o resultado é ignorado.
    saida.push_str("{\"traceEvents\":[\n");
    saida.push_str("{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":1,\"args\":{\"name\":\"principal\"}}");
    for e in p.eventos.iter() {
        let nome = p.nomes.get(usize::from(e.zona)).copied().unwrap_or("?");
        let fase = if e.tipo == EVENTO_INICIO { "B" } else { "E" };
        let ts = e.tsc.saturating_sub(p.base_tsc) as f64 * micro_por_tick;
        saida.push_str(",\n{\"name\":");
        json_texto(saida, nome);
        let _ = write!(saida, ",\"ph\":\"{fase}\",\"ts\":{ts:.3},\"pid\":1,\"tid\":1}}");
    }
    let _ = write!(
        saida,
        "\n],\"displayTimeUnit\":\"ms\",\"otherData\":{{\"eventos_perdidos\":{}}}}}\n",
        p.perdidos
    );
}

fn json_texto(saida: &mut String, s: &str) {
    saida.push('"');
    for c in s.chars() {
        match c {
            '"' => saida.push_str("\\\""),
            '\\' => saida.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(saida, "\\u{:04x}", u32::from(c));
            }
            c => saida.push(c),
        }
    }
    saida.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONA_A: u16 = 0;
    const ZONA_B: u16 = 1;
    static NOMES: [&str; 2] = ["frame", "com \"aspas\""];

    #[test]
    fn exporta_pares_inicio_fim() {
        let mut p = profiler_create(&NOMES, 16);
        prof_inicio(&mut p, ZONA_A);
        prof_inicio(&mut p, ZONA_B);
        prof_fim(&mut p, ZONA_B);
        prof_fim(&mut p, ZONA_A);

        let mut s = String::new();
        prof_exportar_chrome_trace(&p, 1.0e9, &mut s);
        if !ATIVO {
            assert!(p.eventos.is_empty());
            assert!(!s.contains("\"ph\":\"B\""));
            return;
        }
        assert_eq!(p.eventos.len(), 4);
        assert_eq!(s.matches("\"ph\":\"B\"").count(), 2);
        assert_eq!(s.matches("\"ph\":\"E\"").count(), 2);
        assert!(s.contains("\"name\":\"frame\""));
        assert!(s.contains("\"name\":\"com \\\"aspas\\\"\""));
        assert!(s.contains("\"eventos_perdidos\":0"));
    }

    #[test]
    fn buffer_cheio_descarta_e_conta() {
        let mut p = profiler_create(&NOMES, 2);
        prof_inicio(&mut p, ZONA_A);
        prof_fim(&mut p, ZONA_A);
        prof_inicio(&mut p, ZONA_A);
        if ATIVO {
            assert_eq!(p.eventos.len(), 2);
            assert_eq!(p.perdidos, 1);
        } else {
            assert_eq!(p.perdidos, 0);
        }
    }

    #[test]
    fn relogio_avanca() {
        let a = prof_relogio();
        let mut x = 0u64;
        for i in 0..10_000u64 {
            x = x.wrapping_add(i);
        }
        assert!(x > 0);
        assert!(prof_relogio() >= a);
    }
}
