//! Log simples em stderr, sem alocar: recebe `format_args!(...)`, que formata direto na saída.
//!
//! `log_info(format_args!("{n} eventos"))`

use core::fmt;

pub fn log_info(args: fmt::Arguments) {
    eprintln!("[info] {args}");
}

pub fn log_aviso(args: fmt::Arguments) {
    eprintln!("[aviso] {args}");
}

pub fn log_erro(args: fmt::Arguments) {
    eprintln!("[erro] {args}");
}
