//! Log em stderr, sem alocar: recebe `format_args!(...)`.
//!
//! `log::info(format_args!("{n} eventos"))`
//!
//! O log não vai para o crash report (pode ter dados do jogador). Linhas maiores que
//! `LINE_MAX` são cortadas.

use core::fmt::{self, Write};

use forja_core::text::TextBuf;

pub const LINE_MAX: usize = 1024;

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
}
