//! Log em stderr, sem alocar: recebe `format_args!(...)`.
//!
//! `log::info(format_args!("{n} eventos"))`
//!
//! Cada linha também vai para os breadcrumbs: um ring buffer fixo com as últimas linhas, que
//! o crash report inclui (como o log de crash da Unreal). Linhas maiores que `LINE_MAX` são
//! cortadas.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};

use forja_core::text::TextBuf;

use crate::spin_lock::SpinLock;

pub const LINE_MAX: usize = 1024;
pub const BREADCRUMB_CAPACITY: usize = 16 * 1024;

struct RingData {
    buf: [u8; BREADCRUMB_CAPACITY],
    pos: usize,
    wrapped: bool,
}

struct Breadcrumbs {
    lock: SpinLock,
    data: UnsafeCell<RingData>,
}

// SAFETY: `data` só é acessado com `lock` pego (ou, no crash, por uma única thread que
// aceita ler um estado parcial).
unsafe impl Sync for Breadcrumbs {}

static BREADCRUMBS: Breadcrumbs = Breadcrumbs {
    lock: SpinLock::new(),
    data: UnsafeCell::new(RingData { buf: [0; BREADCRUMB_CAPACITY], pos: 0, wrapped: false }),
};

pub fn info(args: fmt::Arguments) {
    write_line("info", args);
}

pub fn warn(args: fmt::Arguments) {
    write_line("aviso", args);
}

pub fn error(args: fmt::Arguments) {
    write_line("erro", args);
}

fn write_line(level: &str, args: fmt::Arguments) {
    let mut buf = [0u8; LINE_MAX];
    let mut t = TextBuf::new(&mut buf);
    let _ = write!(t, "[{level}] {args}");
    if t.truncated() {
        let _ = t.write_str(" [...]");
    }
    let _ = t.write_char('\n');
    let _ = std::io::Write::write_all(&mut std::io::stderr(), t.as_bytes());
    breadcrumbs_append(t.as_bytes());
}

fn breadcrumbs_append(bytes: &[u8]) {
    if !BREADCRUMBS.lock.try_lock(100_000) {
        return;
    }
    // SAFETY: lock pego.
    let d = unsafe { &mut *BREADCRUMBS.data.get() };
    for &b in bytes {
        if let Some(x) = d.buf.get_mut(d.pos) {
            *x = b;
        }
        d.pos = d.pos.saturating_add(1);
        if d.pos >= BREADCRUMB_CAPACITY {
            d.pos = 0;
            d.wrapped = true;
        }
    }
    BREADCRUMBS.lock.unlock();
}

/// Escreve os breadcrumbs em ordem (mais antigo primeiro). Usado pelo crash report: se o lock
/// estiver preso (a thread morreu no meio de um log), lê assim mesmo.
pub(crate) fn write_breadcrumbs(t: &mut TextBuf) {
    let locked = BREADCRUMBS.lock.try_lock(1000);
    // SAFETY: com o lock, ou no crash (uma thread só escreve o relatório); pior caso,
    // uma linha sai embaralhada.
    let d = unsafe { &*BREADCRUMBS.data.get() };
    let (older, newer) = if d.wrapped {
        (d.buf.get(d.pos..).unwrap_or(&[]), d.buf.get(..d.pos).unwrap_or(&[]))
    } else {
        (d.buf.get(..d.pos).unwrap_or(&[]), &[][..])
    };
    // Quando o anel deu a volta, a primeira linha está cortada: pula até o primeiro '\n'.
    let older = if d.wrapped {
        match older.iter().position(|&b| b == b'\n') {
            Some(i) => older.get(i.saturating_add(1)..).unwrap_or(&[]),
            None => &[][..],
        }
    } else {
        older
    };
    for part in [older, newer] {
        for chunk in part.utf8_chunks() {
            let _ = t.write_str(chunk.valid());
        }
    }
    if locked {
        BREADCRUMBS.lock.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breadcrumbs_keep_the_latest_lines() {
        // Único teste que mexe nos breadcrumbs globais. Chama `breadcrumbs_append` direto
        // para não encher o stderr dos testes.
        for i in 0..2000u32 {
            let mut b = [0u8; 64];
            let mut l = TextBuf::new(&mut b);
            let _ = writeln!(l, "[info] linha {i}");
            breadcrumbs_append(l.as_bytes());
        }
        let mut buf = vec![0u8; BREADCRUMB_CAPACITY + 64];
        let mut t = TextBuf::new(&mut buf);
        write_breadcrumbs(&mut t);
        let s = t.as_str();
        assert!(s.ends_with("[info] linha 1999\n"));
        assert!(s.starts_with("[info] linha "));
        assert!(!s.contains("linha 0\n"));
    }
}
