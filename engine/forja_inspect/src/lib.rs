//! Profiler por zonas e contadores. Grava eventos num slice fornecido por quem chama
//! (normalmente da arena persistent) e exporta no formato Chrome Trace (JSON), que abre em
//! https://ui.perfetto.dev. Não aloca.
//!
//! - `Zone` e `Counter` são índices numa tabela de nomes que o jogo define; tipos diferentes
//!   para não passar um contador onde se espera uma zona.
//! - Relógio: `rdtsc` (poucos ns por evento), calibrado contra `Instant` na exportação.
//! - Uma thread só por enquanto (todos os eventos vão para `tid` 1).
//! - Buffer cheio: eventos novos são descartados e contados em `dropped()`.
//!
//! Sem a feature `enabled`, `ENABLED` é `false` e os métodos de registro não fazem nada.

use core::fmt::Write;
use std::time::Instant;

use forja_core::text::TextBuf;

pub const ENABLED: bool = cfg!(feature = "enabled");

/// Zona de tempo (início/fim). Índice na tabela de nomes do profiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone(pub u16);

/// Contador (valor num instante). Índice na tabela de nomes do profiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counter(pub u16);

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Begin = 0,
    End = 1,
    Counter = 2,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfEvent {
    pub tsc: u64,
    /// Só para `EventKind::Counter`.
    pub value: u32,
    /// Índice na tabela de nomes (de uma `Zone` ou de um `Counter`).
    pub name: u16,
    pub kind: EventKind,
}

impl ProfEvent {
    pub const EMPTY: ProfEvent = ProfEvent { tsc: 0, value: 0, name: 0, kind: EventKind::Begin };
}

pub struct Profiler<'a> {
    names: &'static [&'static str],
    events: &'a mut [ProfEvent],
    used: usize,
    dropped: u64,
    base_instant: Instant,
    base_tsc: u64,
}

#[cfg(target_arch = "x86_64")]
fn read_clock() -> u64 {
    // SAFETY: rdtsc só lê o contador da CPU; todo x86_64 suporta.
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(target_arch = "x86_64"))]
fn read_clock() -> u64 {
    match std::time::UNIX_EPOCH.elapsed() {
        Ok(d) => u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

impl<'a> Profiler<'a> {
    /// `events` é o buffer (16 bytes por evento). Com `ENABLED == false`, passe um slice vazio.
    pub fn new(names: &'static [&'static str], events: &'a mut [ProfEvent]) -> Profiler<'a> {
        Profiler { names, events, used: 0, dropped: 0, base_instant: Instant::now(), base_tsc: read_clock() }
    }

    fn record(&mut self, name: u16, kind: EventKind, value: u32) {
        if !ENABLED {
            return;
        }
        let Some(e) = self.events.get_mut(self.used) else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        *e = ProfEvent { tsc: read_clock(), value, name, kind };
        self.used = self.used.saturating_add(1);
    }

    pub fn begin(&mut self, zone: Zone) {
        self.record(zone.0, EventKind::Begin, 0);
    }

    pub fn end(&mut self, zone: Zone) {
        self.record(zone.0, EventKind::End, 0);
    }

    /// Valor de um contador neste instante (ex.: bytes usados na arena do frame).
    pub fn counter(&mut self, counter: Counter, value: u32) {
        self.record(counter.0, EventKind::Counter, value);
    }

    /// Eventos gravados.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Eventos descartados por buffer cheio.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Frequência do relógio medida desde `new`. Quanto mais tempo rodou, mais precisa.
    pub fn tsc_per_second(&self) -> f64 {
        let seconds = self.base_instant.elapsed().as_secs_f64();
        let ticks = read_clock().saturating_sub(self.base_tsc);
        if seconds <= 0.0 {
            return 1.0;
        }
        ticks as f64 / seconds
    }
}

// ---------------------------------------------------------------- exportação

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportStage {
    Header,
    Events,
    Footer,
    Done,
}

/// Maior linha JSON gerada (um evento). Nomes que estourarem saem cortados.
const LINE_MAX: usize = 512;

/// Exportação para Chrome Trace em pedaços: chame `next_chunk` em loop, gravando cada pedaço,
/// até devolver 0. Não precisa de memória além do buffer de quem chama.
pub struct ChromeTraceExport {
    stage: ExportStage,
    next_event: usize,
    micros_per_tick: f64,
}

impl ChromeTraceExport {
    pub fn new(tsc_per_second: f64) -> ChromeTraceExport {
        let micros_per_tick = if tsc_per_second > 0.0 { 1.0e6 / tsc_per_second } else { 0.0 };
        ChromeTraceExport { stage: ExportStage::Header, next_event: 0, micros_per_tick }
    }

    /// Preenche `out` com o próximo pedaço do JSON e devolve quantos bytes escreveu.
    /// `0` significa que terminou.
    pub fn next_chunk(&mut self, p: &Profiler, out: &mut [u8]) -> usize {
        let mut written: usize = 0;
        while self.stage != ExportStage::Done {
            if self.stage == ExportStage::Events && self.next_event >= p.used {
                self.stage = ExportStage::Footer;
            }
            let mut line_buf = [0u8; LINE_MAX];
            let mut line = TextBuf::new(&mut line_buf);
            self.write_line(p, &mut line);
            let bytes = line.as_bytes();
            let end = written.saturating_add(bytes.len());
            match out.get_mut(written..end) {
                Some(dst) => {
                    dst.copy_from_slice(bytes);
                    written = end;
                }
                // Não coube: fica para o próximo pedaço.
                None if written > 0 => return written,
                // Maior que `out` inteiro: descarta para não travar.
                None => {}
            }
            self.advance();
        }
        written
    }

    fn advance(&mut self) {
        match self.stage {
            ExportStage::Header => self.stage = ExportStage::Events,
            ExportStage::Events => self.next_event = self.next_event.saturating_add(1),
            ExportStage::Footer | ExportStage::Done => self.stage = ExportStage::Done,
        }
    }

    // `write!` em `TextBuf` nunca falha (corta o excesso); os resultados são ignorados.
    fn write_line(&self, p: &Profiler, line: &mut TextBuf) {
        match self.stage {
            ExportStage::Header => {
                let _ = line.write_str(
                    "{\"traceEvents\":[\n\
                     {\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":1,\"args\":{\"name\":\"main\"}}",
                );
            }
            ExportStage::Events => {
                let Some(e) = p.events.get(self.next_event) else {
                    return;
                };
                let name = p.names.get(usize::from(e.name)).copied().unwrap_or("?");
                let ts = e.tsc.saturating_sub(p.base_tsc) as f64 * self.micros_per_tick;
                let _ = line.write_str(",\n{\"name\":");
                write_json_string(line, name);
                let _ = match e.kind {
                    EventKind::Counter => write!(
                        line,
                        ",\"ph\":\"C\",\"ts\":{ts:.3},\"pid\":1,\"tid\":1,\"args\":{{\"value\":{}}}}}",
                        e.value
                    ),
                    EventKind::Begin => write!(line, ",\"ph\":\"B\",\"ts\":{ts:.3},\"pid\":1,\"tid\":1}}"),
                    EventKind::End => write!(line, ",\"ph\":\"E\",\"ts\":{ts:.3},\"pid\":1,\"tid\":1}}"),
                };
            }
            ExportStage::Footer => {
                let _ = write!(
                    line,
                    "\n],\"displayTimeUnit\":\"ms\",\"otherData\":{{\"dropped_events\":{}}}}}\n",
                    p.dropped
                );
            }
            ExportStage::Done => {}
        }
    }
}

fn write_json_string(out: &mut TextBuf, s: &str) {
    let _ = out.write_char('"');
    for c in s.chars() {
        let _ = match c {
            '"' => out.write_str("\\\""),
            '\\' => out.write_str("\\\\"),
            c if u32::from(c) < 0x20 => write!(out, "\\u{:04x}", u32::from(c)),
            c => out.write_char(c),
        };
    }
    let _ = out.write_char('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONE_A: Zone = Zone(0);
    const ZONE_B: Zone = Zone(1);
    const COUNTER_C: Counter = Counter(2);
    static NAMES: [&str; 3] = ["frame", "com \"aspas\"", "mem"];

    fn export(p: &Profiler, chunk: usize) -> String {
        let mut exp = ChromeTraceExport::new(1.0e9);
        let mut buf = vec![0u8; chunk];
        let mut all = Vec::new();
        loop {
            let n = exp.next_chunk(p, &mut buf);
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(all).unwrap()
    }

    #[test]
    fn exports_zones_and_counters_in_chunks() {
        let mut ev = [ProfEvent::EMPTY; 16];
        let mut p = Profiler::new(&NAMES, &mut ev);
        p.begin(ZONE_A);
        p.begin(ZONE_B);
        p.end(ZONE_B);
        p.counter(COUNTER_C, 4096);
        p.end(ZONE_A);

        // pedaço pequeno força várias chamadas; o resultado deve ser idêntico ao pedaço grande
        let s = export(&p, 200);
        assert_eq!(s, export(&p, 1 << 16));
        if !ENABLED {
            assert_eq!(p.used(), 0);
            assert!(!s.contains("\"ph\":\"B\""));
            return;
        }
        assert_eq!(p.used(), 5);
        assert_eq!(s.matches("\"ph\":\"B\"").count(), 2);
        assert_eq!(s.matches("\"ph\":\"E\"").count(), 2);
        assert!(s.contains("\"name\":\"mem\",\"ph\":\"C\""));
        assert!(s.contains("\"args\":{\"value\":4096}"));
        assert!(s.contains("\"name\":\"com \\\"aspas\\\"\""));
        assert!(s.ends_with("\"dropped_events\":0}}\n"));
    }

    #[test]
    fn chunk_smaller_than_a_line_terminates() {
        let mut ev = [ProfEvent::EMPTY; 4];
        let mut p = Profiler::new(&NAMES, &mut ev);
        p.begin(ZONE_A);
        let mut exp = ChromeTraceExport::new(1.0e9);
        let mut buf = [0u8; 8];
        let mut calls = 0u32;
        while exp.next_chunk(&p, &mut buf) != 0 {
            calls = calls.saturating_add(1);
            assert!(calls < 10);
        }
    }

    #[test]
    fn full_buffer_drops_and_counts() {
        let mut ev = [ProfEvent::EMPTY; 2];
        let mut p = Profiler::new(&NAMES, &mut ev);
        p.begin(ZONE_A);
        p.end(ZONE_A);
        p.begin(ZONE_A);
        if ENABLED {
            assert_eq!(p.used(), 2);
            assert_eq!(p.dropped(), 1);
        } else {
            assert_eq!(p.dropped(), 0);
        }
    }
}
