//! Contrato com o sistema operacional.
//!
//! O contrato e dividido por categoria de funcionalidade do SO; cada uma e
//! uma trait no seu modulo:
//!
//! - [`process`]: ciclo de vida do processo.
//! - [`memory`]: paginas de memoria virtual e heap do processo.
//! - [`thread`]: criacao (com nucleo, prioridade e stack), espera e controle
//!   de threads.
//! - [`sync`]: semaforo, mutex leve e espera em endereco.
//! - [`cpu`]: topologia da CPU (nucleos, SMT, caches).
//! - [`time`]: relogio monotonico de alta resolucao e espera precisa.
//! - [`window`]: superficie de desenho, input de teclado e mouse, eventos do
//!   sistema (foco, suspensao, pedido de encerrar).
//! - [`input`]: teclado, mouse e gamepads como estado por frame.
//! - [`audio`]: saida de som, com o jogo empurrando amostras.
//! - [`file`]: leitura de arquivos e, no desktop, caminhos arbitrarios.
//! - [`storage`]: conteudo do jogo e saves, sem caminhos.
//! - [`diagnostics`]: saida de texto (console, debugger) e controle do
//!   debugger.
//! - [`dialog`]: caixas de dialogo modais do sistema (desktop).
//! - [`library`]: bibliotecas dinamicas, para hot reload e APIs carregadas em
//!   runtime (desktop).
//!
//! # Nucleo e desktop
//!
//! [`Os`] e o nucleo: o que toda plataforma tem, de um PC a um console
//! fechado. Todo backend implementa tudo, e o codigo do jogo deveria usar
//! so isto. `use forge_os::prelude::*;` traz o nucleo.
//!
//! [`Desktop`] sao capacidades que so existem num SO de janelas: tela cheia
//! e DPI, caixas de mensagem, caminhos arbitrarios, bibliotecas dinamicas.
//! Fica fora do `Os` de proposito: um console nao implementa, e codigo que
//! depende disso precisa pedir com `use forge_os::desktop::*;`. Num alvo sem
//! `Desktop`, esse codigo nao compila, e o erro aponta exatamente o que
//! precisa de alternativa. Ferramentas, editores e hot reload vivem aqui.
//!
//! Acima do contrato, [`forge_log!`] formata texto sem `std` e escreve via
//! `diagnostics`.
//!
//! Cada backend implementa o `Os` (e o `Desktop`, se for desktop) num tipo
//! concreto, e `Platform` aponta para o backend do alvo atual. Hoje so
//! `win32` esta ativo; `linux` esta desabilitado ate definirmos a API grafica.
//!
//! A selecao e feita em tempo de compilacao: sem `dyn`, sem vtable, sem
//! generics vazando para quem chama. `Platform::time_ticks()` compila
//! para uma chamada direta a funcao do backend.
//!
//! As funcoes pertencem as traits de categoria, que precisam estar em escopo:
//! `use forge_os::prelude::*;` traz as do nucleo junto com `Platform`.
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

pub mod audio;
pub mod cpu;
pub mod diagnostics;
pub mod dialog;
pub mod file;
pub mod input;
pub mod library;
pub mod log;
pub mod memory;
pub mod process;
pub mod storage;
pub mod sync;
pub mod thread;
pub mod time;
pub mod window;

/// O nucleo: tudo o que o jogo pode pedir ao SO, em qualquer plataforma. Um
/// backend so e `Os` se implementar todas as categorias; faltando uma, nao
/// compila.
pub trait Os:
    process::Process
    + memory::Memory
    + thread::Thread
    + sync::Synchronization
    + cpu::Cpu
    + time::Time
    + window::Window
    + input::Gamepads
    + audio::Audio
    + file::Files
    + storage::Storage
    + diagnostics::Diagnostics
{
}

/// Capacidades de desktop (Windows, Linux, Mac), fora do nucleo: um console
/// nao implementa.
pub trait Desktop: window::DesktopWindow + dialog::Dialog + file::FileSystem + library::Library {}

/// O nucleo: `Platform`, as traits do `Os`, os tipos delas e `forge_log!`.
pub mod prelude {
    pub use crate::audio::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE, Audio, AudioHandle};
    pub use crate::cpu::{Cpu, CpuTopology};
    pub use crate::diagnostics::Diagnostics;
    pub use crate::file::{FileHandle, FileView, Files};
    pub use crate::forge_log;
    pub use crate::input::{
        Button, Gamepad, GamepadButton, Gamepads, Input, Key, Keyboard, Mouse, MouseButton,
    };
    pub use crate::memory::{Memory, PageInfo};
    pub use crate::process::Process;
    pub use crate::storage::{SaveHandle, Storage};
    pub use crate::sync::{Mutex, SemaphoreHandle, Synchronization};
    pub use crate::thread::{Thread, ThreadEntry, ThreadHandle, ThreadOptions, ThreadPriority};
    pub use crate::time::{Time, TimerHandle};
    pub use crate::window::{AppState, NativeWindow, Window, WindowHandle};
    pub use crate::{Os, Platform};
}

/// As capacidades de desktop, para quem depende delas declarar:
/// `use forge_os::desktop::*;`.
pub mod desktop {
    pub use crate::Desktop;
    pub use crate::dialog::Dialog;
    pub use crate::file::{EntryKind, FileAccess, FileSystem};
    pub use crate::library::{Library, LibraryHandle};
    pub use crate::window::DesktopWindow;
}

#[cfg(windows)]
mod win32;

#[cfg(windows)]
pub type Platform = win32::Win32;

#[cfg(not(windows))]
compile_error!("forge_os: so o backend win32 esta ativo.");
