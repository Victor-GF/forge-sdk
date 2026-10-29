//! Log em stderr, sem alocar: recebe `format_args!(...)`.
//!
//! `log_info(format_args!("{n} eventos"))`
//!
//! Cada linha também vai para o rastro: um ring buffer fixo com as últimas linhas, que o
//! crash report inclui (como o log de crash da Unreal). Linhas maiores que `LINHA_MAX` são
//! cortadas.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};

use forja_core::texto::{Texto, texto_bytes, texto_novo};

use crate::trava::{Trava, trava_nova, trava_soltar, trava_tentar};

pub const LINHA_MAX: usize = 1024;
pub const RASTRO_CAPACIDADE: usize = 16 * 1024;

struct RastroDados {
    buf: [u8; RASTRO_CAPACIDADE],
    pos: usize,
    cheio: bool,
}

struct Rastro {
    trava: Trava,
    dados: UnsafeCell<RastroDados>,
}

// SAFETY: `dados` só é acessado com `trava` pega (ou, no crash, por uma única thread que
// aceita ler um estado parcial).
unsafe impl Sync for Rastro {}

static RASTRO: Rastro = Rastro {
    trava: trava_nova(),
    dados: UnsafeCell::new(RastroDados { buf: [0; RASTRO_CAPACIDADE], pos: 0, cheio: false }),
};

pub fn log_info(args: fmt::Arguments) {
    log_linha("info", args);
}

pub fn log_aviso(args: fmt::Arguments) {
    log_linha("aviso", args);
}

pub fn log_erro(args: fmt::Arguments) {
    log_linha("erro", args);
}

fn log_linha(nivel: &str, args: fmt::Arguments) {
    let mut buf = [0u8; LINHA_MAX];
    let mut t = texto_novo(&mut buf);
    let _ = write!(t, "[{nivel}] {args}");
    if t.truncou {
        let _ = t.write_str(" [...]");
    }
    let _ = t.write_char('\n');
    let bytes = texto_bytes(&t);
    let _ = std::io::Write::write_all(&mut std::io::stderr(), bytes);
    rastro_anexar(bytes);
}

fn rastro_anexar(bytes: &[u8]) {
    if !trava_tentar(&RASTRO.trava, 100_000) {
        return;
    }
    // SAFETY: trava pega.
    let d = unsafe { &mut *RASTRO.dados.get() };
    for &b in bytes {
        if let Some(x) = d.buf.get_mut(d.pos) {
            *x = b;
        }
        d.pos = d.pos.saturating_add(1);
        if d.pos >= RASTRO_CAPACIDADE {
            d.pos = 0;
            d.cheio = true;
        }
    }
    trava_soltar(&RASTRO.trava);
}

/// Escreve o rastro em ordem (mais antiga primeiro). Usado pelo crash report: se a trava
/// estiver presa (a thread morreu no meio de um log), lê assim mesmo.
pub(crate) fn rastro_escrever(t: &mut Texto) {
    let pegou = trava_tentar(&RASTRO.trava, 1000);
    // SAFETY: com a trava, ou no crash (uma thread só escreve o relatório); pior caso,
    // uma linha sai embaralhada.
    let d = unsafe { &*RASTRO.dados.get() };
    let (antigo, novo) = if d.cheio {
        (d.buf.get(d.pos..).unwrap_or(&[]), d.buf.get(..d.pos).unwrap_or(&[]))
    } else {
        (d.buf.get(..d.pos).unwrap_or(&[]), &[][..])
    };
    // Quando o anel deu a volta, a primeira linha está cortada: pula até o primeiro '\n'.
    let antigo = if d.cheio {
        match antigo.iter().position(|&b| b == b'\n') {
            Some(i) => antigo.get(i.saturating_add(1)..).unwrap_or(&[]),
            None => &[][..],
        }
    } else {
        antigo
    };
    for parte in [antigo, novo] {
        for pedaco in parte.utf8_chunks() {
            let _ = t.write_str(pedaco.valid());
        }
    }
    if pegou {
        trava_soltar(&RASTRO.trava);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forja_core::texto::texto_str;

    #[test]
    fn rastro_guarda_as_linhas_mais_recentes() {
        // Único teste que mexe no rastro global. Chama `rastro_anexar` direto para não
        // encher o stderr dos testes.
        for i in 0..2000u32 {
            let mut b = [0u8; 64];
            let mut l = texto_novo(&mut b);
            let _ = writeln!(l, "[info] linha {i}");
            rastro_anexar(texto_bytes(&l));
        }
        let mut buf = vec![0u8; RASTRO_CAPACIDADE + 64];
        let mut t = texto_novo(&mut buf);
        rastro_escrever(&mut t);
        let s = texto_str(&t);
        assert!(s.ends_with("[info] linha 1999\n"));
        assert!(s.starts_with("[info] linha "));
        assert!(!s.contains("linha 0\n"));
    }
}
