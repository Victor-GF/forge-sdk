//! Memória da Forja.
//!
//! - `memoria`: uma única chamada ao SO (`VirtualAlloc`) reserva o orçamento do programa.
//!   Dele saem as **mesas**: um conjunto de arenas por thread (persistente, fase, frame).
//! - `arena`: bump allocator de uma thread só. `Send` (pode ir para outra thread),
//!   nunca `Sync` (nunca usado por duas ao mesmo tempo) — o compilador garante.
//! - `global`: alocador global do Rust. Atende só a inicialização da std (buffer fixo em
//!   `.bss`, sem chamar o SO) e depois é **travado**: qualquer `Vec`/`String`/`Box` aborta.
//!
//! Exceção documentada ao estilo: `arena_slice`/`arena_valor` são genéricos (`T: Copy`),
//! para entregar memória tipada sem `unsafe` espalhado pelo jogo.

pub mod arena;
pub mod global;
pub mod memoria;
