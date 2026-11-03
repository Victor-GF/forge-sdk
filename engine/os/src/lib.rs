//! Contrato com o sistema operacional.
//!
//! O contrato e dividido por categoria de funcionalidade do SO; cada uma e
//! uma trait no seu modulo:
//!
//! - [`process`]: ciclo de vida do processo.
//! - [`memory`]: paginas de memoria virtual e heap do processo.
//! - [`thread`]: criacao, espera e controle de threads.
//! - [`sync`]: primitivas de sincronizacao que bloqueiam no SO.
//! - [`window`]: janela nativa do sistema e seus eventos.
//! - [`dialog`]: caixas de dialogo modais do sistema.
//!
//! `Os` junta todas as categorias. Cada backend implementa todas num tipo
//! concreto, e `Platform` aponta para o backend do alvo atual. Hoje so
//! `win32` esta ativo; `linux` esta desabilitado ate definirmos a API grafica.
//!
//! A selecao e feita em tempo de compilacao: sem `dyn`, sem vtable, sem
//! generics vazando para quem chama. `Platform::message_box(...)` compila
//! para uma chamada direta a funcao do backend.
//!
//! As funcoes pertencem as traits de categoria, que precisam estar em escopo:
//! `use forge_os::prelude::*;` traz todas junto com `Platform`.

#![no_std]

pub mod dialog;
pub mod memory;
pub mod process;
pub mod sync;
pub mod thread;
pub mod window;

/// Tudo o que a engine pode pedir ao SO. Um backend so e `Os` se implementar
/// todas as categorias; faltando uma, nao compila.
pub trait Os:
    process::Process + memory::Memory + thread::Thread + sync::Synchronization + window::Window + dialog::Dialog
{
}

/// `Platform` e as traits de todas as categorias.
pub mod prelude {
    pub use crate::dialog::Dialog;
    pub use crate::memory::{Memory, PageInfo};
    pub use crate::process::Process;
    pub use crate::sync::{SemaphoreHandle, Synchronization};
    pub use crate::thread::{Thread, ThreadEntry, ThreadHandle};
    pub use crate::window::{Window, WindowHandle};
    pub use crate::{Os, Platform};
}

#[cfg(windows)]
mod win32;

#[cfg(windows)]
pub type Platform = win32::Win32;

#[cfg(not(windows))]
compile_error!("forge_os: so o backend win32 esta ativo.");
