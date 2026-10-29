//! Região do programa: um único `VirtualAlloc` com o orçamento total, recortado em arenas.
//!
//! Páginas: o SO entrega memória em páginas (4 KB) dentro de blocos de reserva (64 KB).
//! A região é arredondada para o bloco e cada arena começa numa página e tem tamanho múltiplo
//! de página. O arredondamento não é desperdício: os bytes extras ficam com a própria arena.
//! Dentro da arena não há alinhamento a página: os dados ficam colados.
//!
//! A região nunca é devolvida ao SO (vive até o processo acabar); por isso as arenas
//! recortadas dela podem ser movidas livremente sem risco de apontar para memória liberada.

use core::ptr::null;
use core::sync::atomic::{AtomicBool, Ordering};

use forja_diag::crash::diag_registrar_secao;
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

use crate::arena::{Arena, arena_de_bytes, bump_reservar};
use crate::painel::{REGIAO_CAPACIDADE, REGIAO_PAGINA, REGIAO_USADO, mem_secao_crash};

/// Dona da região. Fica na thread principal; o que sai dela (arenas, mesas) pode ir para
/// outras threads.
pub struct Memoria {
    base: *mut u8,
    capacidade: usize,
    usado: usize,
    pagina: usize,
}

/// Orçamento de uma mesa, em bytes (cada arena é arredondada para cima até a página).
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

static SECAO_REGISTRADA: AtomicBool = AtomicBool::new(false);

/// Arredonda `v` para cima até múltiplo de `m` (potência de 2).
pub fn arredondar_potencia2(v: usize, m: usize) -> Option<usize> {
    if !m.is_power_of_two() {
        return None;
    }
    let mascara = m.wrapping_sub(1);
    Some(v.checked_add(mascara)? & !mascara)
}

/// (tamanho da página, granularidade de reserva) do SO.
pub fn sistema_paginas() -> (usize, usize) {
    // SAFETY: SYSTEM_INFO só tem inteiros e ponteiros; zero é válido; a função só escreve.
    let info: SYSTEM_INFO = unsafe {
        let mut i = core::mem::zeroed();
        GetSystemInfo(&mut i);
        i
    };
    (info.dwPageSize as usize, info.dwAllocationGranularity as usize)
}

/// Pede ao SO `capacidade` bytes (reservados e commitados), arredondados para o bloco de
/// reserva. Chame uma vez, no início. Páginas nunca tocadas não ocupam RAM física (mas
/// contam no limite de commit do sistema).
pub fn memoria_init(capacidade: usize) -> Option<Memoria> {
    let (pagina, granularidade) = sistema_paginas();
    let capacidade = arredondar_potencia2(capacidade, granularidade)?;
    // SAFETY: pedido simples de memória nova; o resultado é checado.
    let base = unsafe { VirtualAlloc(null(), capacidade, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
    if base.is_null() {
        return None;
    }
    REGIAO_CAPACIDADE.store(capacidade, Ordering::Relaxed);
    REGIAO_PAGINA.store(pagina, Ordering::Relaxed);
    if !SECAO_REGISTRADA.swap(true, Ordering::AcqRel) {
        diag_registrar_secao("memória da forja", mem_secao_crash);
    }
    Some(Memoria { base: base.cast::<u8>(), capacidade, usado: 0, pagina })
}

/// Recorta uma arena começando numa página, com capacidade arredondada para página.
pub fn memoria_arena(m: &mut Memoria, nome: &'static str, capacidade: usize) -> Option<Arena> {
    let capacidade = arredondar_potencia2(capacidade, m.pagina)?;
    let (inicio, fim) = bump_reservar(m.base as usize, m.usado, capacidade, m.pagina, m.capacidade)?;
    // SAFETY: a faixa [inicio, fim) está dentro da região, é exclusiva (o cursor avança logo
    // abaixo) e a região nunca é liberada.
    let arena = unsafe { arena_de_bytes(nome, m.base.wrapping_add(inicio), capacidade) }?;
    m.usado = fim;
    REGIAO_USADO.store(fim, Ordering::Relaxed);
    Some(arena)
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

pub fn memoria_pagina(m: &Memoria) -> usize {
    m.pagina
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // testes seguem as mesmas exceções do clippy.toml
mod tests {
    use super::*;
    use crate::arena::{arena_estatistica, arena_slice, arena_usado};

    #[test]
    fn arredonda_para_potencia_de_2() {
        assert_eq!(arredondar_potencia2(1, 4096), Some(4096));
        assert_eq!(arredondar_potencia2(4096, 4096), Some(4096));
        assert_eq!(arredondar_potencia2(4097, 4096), Some(8192));
        assert_eq!(arredondar_potencia2(10, 3), None);
        assert_eq!(arredondar_potencia2(usize::MAX, 4096), None);
    }

    #[test]
    fn regiao_e_arenas_seguem_as_paginas() {
        let (pagina, granularidade) = sistema_paginas();
        let mut m = memoria_init(1000).unwrap();
        assert_eq!(memoria_capacidade(&m), granularidade);
        assert_eq!(memoria_pagina(&m), pagina);

        let desc = MesaDesc { persistente: 1000, fase: 1, frame: pagina + 1 };
        let mesa = memoria_mesa(&mut m, &desc).unwrap();
        // cada arena ganha a página inteira; frame ocupa duas
        assert_eq!(arena_estatistica(&mesa.persistente).capacidade, pagina);
        assert_eq!(arena_estatistica(&mesa.fase).capacidade, pagina);
        assert_eq!(arena_estatistica(&mesa.frame).capacidade, 2 * pagina);
        assert_eq!(memoria_usado(&m), 4 * pagina);

        let s = arena_slice(&mesa.frame, 2 * pagina, 0xABu8).unwrap();
        assert!((s.as_ptr() as usize).is_multiple_of(pagina));
        assert_eq!(s[2 * pagina - 1], 0xAB);
        assert_eq!(arena_usado(&mesa.fase), 0);

        // não cabe mais que a região
        assert!(memoria_arena(&mut m, "grande", granularidade).is_none());
    }
}
