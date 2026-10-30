//! Memória da Forja.
//!
//! - `region`: uma única chamada ao SO (`VirtualAlloc`) reserva o orçamento do programa,
//!   arredondado para páginas. Dela saem as arenas e a `ThreadMemory` de cada thread
//!   (arenas persistent, level, frame). **Todos os dados da engine e do jogo vêm daqui.**
//! - `arena`: bump allocator de uma thread só. `Send` (pode ir para outra thread),
//!   nunca `Sync` (nunca usado por duas ao mesmo tempo) — o compilador garante.
//! - `stats`: estatísticas atômicas de todas as arenas, para o inspetor.
//! - `global`: heap geral (o `#[global_allocator]`), no modelo da Unreal: atende std e
//!   dependências, com contabilidade por tag, orçamento e relatório de cada ponto de chamada
//!   que aloca depois da inicialização. Opcionalmente proibido (`HeapPolicy::Forbid`).
//!
//! Exceção documentada ao estilo: `Arena::alloc_slice`/`alloc_value` são genéricos
//! (`T: Copy`), para entregar memória tipada sem `unsafe` espalhado pelo jogo.

pub mod arena;
pub mod global;
pub mod region;
pub mod stats;
