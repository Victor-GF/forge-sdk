//! Log formatado sem `std` e sem alocacao: `forge_os::log!("x = {x}")`.
//!
//! Formata numa buffer na stack e escreve cada linha inteira, com `\n`, no
//! console e no debugger ([`Diagnostics`]). Linhas de threads diferentes nao
//! se misturam, a menos que uma passe do tamanho da buffer: ai ela sai em
//! pedacos, cortados entre escritas de `fmt`, nunca no meio de um caractere.

use crate::Platform;
use crate::diagnostics::Diagnostics;
use core::fmt::{self, Write};

const CAPACITY: usize = 1024;

/// Formata `format_args!` e escreve como uma linha. Use [`log!`](crate::log!).
#[doc(hidden)]
pub fn write_line(args: fmt::Arguments) {
    let mut line = Line { buf: [0; CAPACITY], len: 0 };
    let _ = line.write_fmt(args);
    let _ = line.write_str("\n");
    line.flush();
}

/// Escreve uma linha formatada no console e no debugger, como `println!`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write_line(::core::format_args!($($arg)*))
    };
}

struct Line {
    buf: [u8; CAPACITY],
    len: usize,
}

impl Line {
    fn flush(&mut self) {
        if self.len == 0 {
            return;
        }
        // So recebe `&str` inteiros em `write_str`: o conteudo e UTF-8.
        let text = unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) };
        emit(text);
        self.len = 0;
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.len + s.len() > CAPACITY {
            self.flush();
        }
        if s.len() > CAPACITY {
            emit(s);
        } else {
            self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
            self.len += s.len();
        }
        Ok(())
    }
}

#[inline]
fn emit(text: &str) {
    Platform::console_write(text);
    Platform::debug_write(text);
}
