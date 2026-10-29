//! Demonstra o bloqueio: depois de `global_travar`, um `Vec` aborta o programa com aviso.
//!
//! cargo run -p forja_mem --example travado

use std::hint::black_box;

use forja_mem::global::{AlocadorGlobal, global_estatistica, global_travar};

#[global_allocator]
static ALOCADOR: AlocadorGlobal = AlocadorGlobal;

fn main() {
    let antes = black_box(vec![1u8, 2, 3]);
    eprintln!("antes de travar: {} bytes usados, vec com {} itens", global_estatistica().usado, antes.len());
    global_travar();
    eprintln!("travado; criando um Vec (deve abortar)...");
    let depois = black_box(Vec::<u8>::with_capacity(100));
    eprintln!("ERRO: não abortou ({} de capacidade)", depois.capacity());
}
