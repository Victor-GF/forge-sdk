//! Backend Win32: implementa o contrato `Os` sobre as chamadas de `ffi`.
//!
//! Uma implementacao por categoria, cada uma no seu arquivo. Todo
//! `extern "system"` continua em `ffi`.

#[cfg(not(target_arch = "x86_64"))]
compile_error!("backend win32 so suporta x86_64: os layouts de `ffi` sao os do Win64.");

mod ffi;

mod dialog;
mod memory;
mod process;
mod sync;
mod thread;
mod window;

pub struct Win32;

impl crate::Os for Win32 {}
