//! Runtime: dono do processo.
//!
//! Inicializa o SO via `forge_os`, roda o loop principal e chama o jogo.
//! Nao fala com o SO diretamente: todo contato passa pelo contrato `Os`.
//!
//! Usa a `main` padrao do Rust: a `std` cuida do startup, do panic e da
//! saida. As bibliotecas da engine continuam `no_std`.

#![windows_subsystem = "windows"]

use forge_os::{Os, Platform};

fn main() {
    Platform::message_box(c"forge", c"Forge SDK");
}
