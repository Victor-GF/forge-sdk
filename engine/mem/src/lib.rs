//! Memória da Forja.
//!
//! - `region`: uma única chamada ao SO (`VirtualAlloc`) reserva o orçamento do programa,
//!   arredondado para páginas. Dela saem as arenas (`region.arena(bytes)`). **Todos os dados
//!   da engine e do jogo vêm daqui.**
//! - `arena`: bump allocator de uma thread só; o estado é só o cursor, dentro dela (sem
//!   atômicos, sem registro global). `Send` (pode ir para outra thread), nunca `Sync` (nunca
//!   usado por duas ao mesmo tempo) — o compilador garante.
//! - `global`: heap geral (o `#[global_allocator]`): repassa ao heap do sistema o que a std e
//!   as dependências pedem e conta alocações e bytes vivos (meta: zero alocações por frame).
//!
//! Exceção documentada ao estilo: `Arena::alloc_slice`/`alloc_value` são genéricos
//! (`T: Copy`), para entregar memória tipada sem `unsafe` espalhado pelo jogo.

pub mod arena;
pub mod global;
pub mod region;
