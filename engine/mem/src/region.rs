//! Região do programa: uma única reserva de memória ao SO (`forja_os::memory::reserve`) com o
//! orçamento total, recortada em arenas.
//!
//! Páginas: o SO entrega memória em páginas (4 KB) dentro de blocos de reserva (64 KB no
//! Windows, uma página no Linux). A região é arredondada para o bloco e cada arena começa numa
//! página e tem tamanho múltiplo de página. O arredondamento não é desperdício: os bytes extras
//! ficam com a própria arena. Dentro da arena não há alinhamento a página: os dados ficam colados.
//!
//! A região nunca é devolvida ao SO (vive até o processo acabar); por isso as arenas
//! recortadas dela podem ser movidas livremente sem risco de apontar para memória liberada.

use forja_os::memory;

pub use forja_os::memory::{PageInfo, page_info};

use crate::arena::{Arena, bump_reserve};

/// Dona da região. Recortar exige `&mut`, então só uma thread recorta; as arenas que saem dela
/// podem ir para outras threads.
pub struct MemoryRegion {
    base: *mut u8,
    capacity: usize,
    used: usize,
    page_size: usize,
}

/// Arredonda `v` para cima até múltiplo de `m` (potência de 2).
pub fn round_up_pow2(v: usize, m: usize) -> Option<usize> {
    if !m.is_power_of_two() {
        return None;
    }
    let mask = m.wrapping_sub(1);
    Some(v.checked_add(mask)? & !mask)
}

impl MemoryRegion {
    /// Pede ao SO `capacity` bytes, arredondados para o bloco de reserva. Chame uma vez, no
    /// início. Páginas nunca tocadas não ocupam RAM física (no Windows contam no limite de
    /// commit do sistema).
    pub fn reserve(capacity: usize) -> Option<MemoryRegion> {
        let pages = page_info();
        let capacity = round_up_pow2(capacity, pages.allocation_granularity)?;
        let base = memory::reserve(capacity)?;
        Some(MemoryRegion { base, capacity, used: 0, page_size: pages.page_size })
    }

    /// Recorta uma arena começando numa página, com capacidade arredondada para página.
    /// `None` se não couber no que sobra da região.
    pub fn arena(&mut self, capacity: usize) -> Option<Arena> {
        let capacity = round_up_pow2(capacity, self.page_size)?;
        let (start, end) = bump_reserve(self.base as usize, self.used, capacity, self.page_size, self.capacity)?;
        // SAFETY: a faixa [start, end) está dentro da região, é exclusiva (o cursor avança logo
        // abaixo) e a região nunca é liberada.
        let arena = unsafe { Arena::from_raw(self.base.wrapping_add(start), capacity) };
        self.used = end;
        Some(arena)
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // testes seguem as mesmas exceções do clippy.toml
mod tests {
    use super::*;

    #[test]
    fn rounds_up_to_power_of_two() {
        assert_eq!(round_up_pow2(1, 4096), Some(4096));
        assert_eq!(round_up_pow2(4096, 4096), Some(4096));
        assert_eq!(round_up_pow2(4097, 4096), Some(8192));
        assert_eq!(round_up_pow2(10, 3), None);
        assert_eq!(round_up_pow2(usize::MAX, 4096), None);
    }

    #[test]
    fn region_and_arenas_follow_pages() {
        // Só o que o contrato garante: granularidade 64 KB no Windows, uma página no Linux.
        let pages = page_info();
        let page = pages.page_size;
        let mut r = MemoryRegion::reserve(4 * page).unwrap();
        assert_eq!(r.capacity(), round_up_pow2(4 * page, pages.allocation_granularity).unwrap());
        assert_eq!(r.page_size(), page);

        // cada arena ganha a página inteira; a segunda ocupa duas
        let small = r.arena(1000).unwrap();
        let big = r.arena(page + 1).unwrap();
        assert_eq!(small.capacity(), page);
        assert_eq!(big.capacity(), 2 * page);
        assert_eq!(r.used(), 3 * page);

        let s = big.alloc_slice(2 * page, 0xABu8).unwrap();
        assert!((s.as_ptr() as usize).is_multiple_of(page));
        assert_eq!(s[2 * page - 1], 0xAB);
        assert_eq!(small.used(), 0);

        // não cabe mais que a região
        assert!(r.arena(r.capacity() - r.used() + 1).is_none());
    }
}
