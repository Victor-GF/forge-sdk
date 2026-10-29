//! Região do programa: um único `VirtualAlloc` com o orçamento total, recortado em arenas.
//!
//! A região nunca é devolvida ao SO (vive até o processo acabar); por isso as arenas
//! recortadas dela podem ser movidas livremente sem risco de apontar para memória liberada.

use core::ptr::null;

use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};

use crate::arena::{Arena, arena_de_bytes, bump_reservar};

/// Alinhamento de cada arena recortada (linha de cache).
pub const MEMORIA_ALINHAMENTO: usize = 64;

/// Dona da região. Fica na thread principal; o que sai dela (arenas, mesas) pode ir para
/// outras threads.
pub struct Memoria {
    base: *mut u8,
    capacidade: usize,
    usado: usize,
}

/// Orçamento de uma mesa, em bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MesaDesc {
    pub persistente: usize,
    pub fase: usize,
    pub frame: usize,
}

/// A "mesa de trabalho" de uma thread: três arenas com vidas diferentes.
/// - `persistente`: o programa inteiro (nunca reseta)
/// - `fase`: reseta ao trocar de fase/pista
/// - `frame`: reseta todo frame
pub struct Mesa {
    pub persistente: Arena,
    pub fase: Arena,
    pub frame: Arena,
}

/// Pede ao SO `capacidade` bytes (reservados e commitados). Chame uma vez, no início.
/// Páginas nunca tocadas não ocupam RAM física.
pub fn memoria_init(capacidade: usize) -> Option<Memoria> {
    // SAFETY: pedido simples de memória nova; o resultado é checado.
    let base = unsafe { VirtualAlloc(null(), capacidade, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
    if base.is_null() {
        return None;
    }
    Some(Memoria { base: base.cast::<u8>(), capacidade, usado: 0 })
}

pub fn memoria_arena(m: &mut Memoria, nome: &'static str, capacidade: usize) -> Option<Arena> {
    let (inicio, fim) =
        bump_reservar(m.base as usize, m.usado, capacidade, MEMORIA_ALINHAMENTO, m.capacidade)?;
    m.usado = fim;
    // SAFETY: a faixa [inicio, fim) está dentro da região, é exclusiva (o cursor avançou)
    // e a região nunca é liberada.
    Some(unsafe { arena_de_bytes(nome, m.base.wrapping_add(inicio), capacidade) })
}

pub fn memoria_mesa(m: &mut Memoria, desc: &MesaDesc) -> Option<Mesa> {
    Some(Mesa {
        persistente: memoria_arena(m, "persistente", desc.persistente)?,
        fase: memoria_arena(m, "fase", desc.fase)?,
        frame: memoria_arena(m, "frame", desc.frame)?,
    })
}

pub fn memoria_usado(m: &Memoria) -> usize {
    m.usado
}

pub fn memoria_capacidade(m: &Memoria) -> usize {
    m.capacidade
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{arena_slice, arena_usado};

    #[test]
    fn recorta_mesas_ate_o_orcamento() {
        let mut m = memoria_init(4096).unwrap();
        let desc = MesaDesc { persistente: 1000, fase: 1000, frame: 1000 };
        let mesa = memoria_mesa(&mut m, &desc).unwrap();
        assert!(memoria_usado(&m) <= memoria_capacidade(&m));
        // a segunda mesa não cabe (alinhamento de 64 entre arenas)
        assert!(memoria_mesa(&mut m, &desc).is_none());

        let s = arena_slice(&mesa.frame, 1000, 0xABu8).unwrap();
        assert_eq!(s[999], 0xAB);
        assert_eq!(arena_usado(&mesa.frame), 1000);
        assert_eq!(arena_usado(&mesa.fase), 0);
    }
}
