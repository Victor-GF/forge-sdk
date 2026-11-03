//! Bindings Win32 escritos a mao.
//!
//! Apenas o que o backend usa, declarado direto contra as DLLs do sistema.
//! Sem `windows-sys`, sem `winapi`: cada simbolo aqui e uma decisao explicita.
//!
//! Regras deste modulo:
//! - Um arquivo por header do Windows SDK, com o mesmo nome. Cada simbolo
//!   fica no arquivo do header que o declara.
//! - Linkagem `raw-dylib`: o rustc gera a import table sozinho, sem depender
//!   das import libs (.a/.lib) do toolchain.
//! - Toda struct `#[repr(C)]` tem seu tamanho e alinhamento travados com
//!   `layout!`, usando os valores x64 do SDK.

#![allow(non_snake_case, non_camel_case_types, dead_code, unused_imports)]

mod basetsd;
mod handleapi;
mod heapapi;
mod libloaderapi;
mod memoryapi;
mod minwinbase;
mod minwindef;
mod processthreadsapi;
mod synchapi;
mod sysinfoapi;
mod winbase;
mod windef;
mod winnt;
mod winuser;

pub use basetsd::*;
pub use handleapi::*;
pub use heapapi::*;
pub use libloaderapi::*;
pub use memoryapi::*;
pub use minwinbase::*;
pub use minwindef::*;
pub use processthreadsapi::*;
pub use synchapi::*;
pub use sysinfoapi::*;
pub use winbase::*;
pub use windef::*;
pub use winnt::*;
pub use winuser::*;
