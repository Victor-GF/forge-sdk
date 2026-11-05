//! Contrato com o sistema operacional.
//!
//! O contrato e dividido por categoria de funcionalidade do SO; cada uma e
//! uma trait no seu modulo:
//!
//! - [`process`]: ciclo de vida do processo.
//! - [`memory`]: paginas de memoria virtual e heap do processo.
//! - [`thread`]: criacao, espera e controle de threads.
//! - [`sync`]: primitivas de sincronizacao que bloqueiam no SO.
//! - [`time`]: relogio monotonico de alta resolucao.
//! - [`window`]: janela nativa do sistema e seus eventos.
//! - [`dialog`]: caixas de dialogo modais do sistema.
//! - [`diagnostics`]: saida de texto (console, debugger) e controle do
//!   debugger.
//! - [`file`]: arquivos, diretorios e caminhos do jogo (executavel, dados
//!   do usuario).
//! - [`input`]: teclado, mouse e gamepads como estado por frame.
//! - [`library`]: bibliotecas dinamicas, para hot reload e APIs carregadas em
//!   runtime.
//!
//! Acima do contrato, [`forge_log!`] formata texto sem `std` e escreve via
//! `diagnostics`.
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
//!
//! # Erros
//!
//! Cada tipo de problema tem um mecanismo, e um backend novo segue o mesmo:
//!
//! - Falha esperada do SO (sem memoria, janela que nao criou, handle
//!   invalido): volta como valor (nulo, `Option`, `bool`). Nunca panic:
//!   quem chama decide se e fatal.
//! - Bug de quem chama ou invariante quebrada: `panic!`/`debug_assert!`. Com
//!   `panic = "abort"` no workspace, panic e um `assert` de C: para o
//!   processo na hora, sem unwinding.
//! - Layout de struct que cruza FFI: `forge_core::layout!`, erro de
//!   compilacao.
//!
//! Panic nao e problema de portabilidade: o mecanismo vive em `core`. O que
//! muda por plataforma e so o que acontece depois, definido pelo
//! `#[panic_handler]` do binario final (o do `std` quando ha `std`; escrito a
//! mao num alvo sem `std`). As libs da engine nao definem esse handler.
//!
//! Callbacks chamados pelo SO (`window_proc`, trampolim de thread) podem
//! entrar em panic: com abort o processo para ali, sem unwinding atravessar
//! a fronteira FFI.

#![no_std]

pub mod diagnostics;
pub mod dialog;
pub mod file;
pub mod input;
pub mod library;
pub mod log;
pub mod memory;
pub mod process;
pub mod sync;
pub mod thread;
pub mod time;
pub mod window;

/// Tudo o que a engine pode pedir ao SO. Um backend so e `Os` se implementar
/// todas as categorias; faltando uma, nao compila.
pub trait Os:
    process::Process
    + memory::Memory
    + thread::Thread
    + sync::Synchronization
    + time::Time
    + window::Window
    + dialog::Dialog
    + diagnostics::Diagnostics
    + file::FileSystem
    + library::Library
    + input::Gamepads
{
}

/// `Platform`, as traits de todas as categorias e `forge_log!`.
pub mod prelude {
    pub use crate::diagnostics::Diagnostics;
    pub use crate::dialog::Dialog;
    pub use crate::file::{EntryKind, FileAccess, FileHandle, FileSystem, FileView};
    pub use crate::forge_log;
    pub use crate::input::{
        Button, Gamepad, GamepadButton, Gamepads, Input, Key, Keyboard, Mouse, MouseButton,
    };
    pub use crate::library::{Library, LibraryHandle};
    pub use crate::memory::{Memory, PageInfo};
    pub use crate::process::Process;
    pub use crate::sync::{SemaphoreHandle, Synchronization};
    pub use crate::thread::{Thread, ThreadEntry, ThreadHandle};
    pub use crate::time::Time;
    pub use crate::window::{Window, WindowHandle};
    pub use crate::{Os, Platform};
}

#[cfg(windows)]
mod win32;

#[cfg(windows)]
pub type Platform = win32::Win32;

#[cfg(not(windows))]
compile_error!("forge_os: so o backend win32 esta ativo.");
