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

use forja_diag::crash;
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

use crate::arena::{Arena, bump_reserve};
use crate::stats::{REGION_CAPACITY, REGION_PAGE_SIZE, REGION_USED, crash_section};

/// Dona da região. Fica na thread principal; o que sai dela (arenas, `ThreadMemory`) pode ir
/// para outras threads.
pub struct MemoryRegion {
    base: *mut u8,
    capacity: usize,
    used: usize,
    page_size: usize,
}

/// Orçamento das arenas de uma thread, em bytes (cada arena é arredondada para cima até a
/// página).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadMemoryDesc {
    pub persistent: usize,
    pub level: usize,
    pub frame: usize,
}

/// As arenas de uma thread, com vidas diferentes:
/// - `persistent`: o programa inteiro (nunca reseta)
/// - `level`: reseta ao trocar de fase/pista
/// - `frame`: reseta todo frame
pub struct ThreadMemory {
    pub persistent: Arena,
    pub level: Arena,
    pub frame: Arena,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageInfo {
    pub page_size: usize,
    /// Granularidade de reserva do `VirtualAlloc` (64 KB no Windows).
    pub allocation_granularity: usize,
}

static CRASH_SECTION_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Arredonda `v` para cima até múltiplo de `m` (potência de 2).
pub fn round_up_pow2(v: usize, m: usize) -> Option<usize> {
    if !m.is_power_of_two() {
        return None;
    }
    let mask = m.wrapping_sub(1);
    Some(v.checked_add(mask)? & !mask)
}

pub fn page_info() -> PageInfo {
    // SAFETY: SYSTEM_INFO só tem inteiros e ponteiros; zero é válido; a função só escreve.
    let info: SYSTEM_INFO = unsafe {
        let mut i = core::mem::zeroed();
        GetSystemInfo(&mut i);
        i
    };
    PageInfo {
        page_size: info.dwPageSize as usize,
        allocation_granularity: info.dwAllocationGranularity as usize,
    }
}

impl MemoryRegion {
    /// Pede ao SO `capacity` bytes (reservados e commitados), arredondados para o bloco de
    /// reserva. Chame uma vez, no início. Páginas nunca tocadas não ocupam RAM física (mas
    /// contam no limite de commit do sistema).
    pub fn reserve(capacity: usize) -> Option<MemoryRegion> {
        let pages = page_info();
        let capacity = round_up_pow2(capacity, pages.allocation_granularity)?;
        // SAFETY: pedido simples de memória nova; o resultado é checado.
        let base = unsafe { VirtualAlloc(null(), capacity, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        if base.is_null() {
            return None;
        }
        REGION_CAPACITY.store(capacity, Ordering::Relaxed);
        REGION_PAGE_SIZE.store(pages.page_size, Ordering::Relaxed);
        if !CRASH_SECTION_REGISTERED.swap(true, Ordering::AcqRel) {
            crash::register_section("memória da forja", crash_section);
        }
        Some(MemoryRegion { base: base.cast::<u8>(), capacity, used: 0, page_size: pages.page_size })
    }

    /// Recorta uma arena começando numa página, com capacidade arredondada para página.
    pub fn arena(&mut self, name: &'static str, capacity: usize) -> Option<Arena> {
        let capacity = round_up_pow2(capacity, self.page_size)?;
        let (start, end) = bump_reserve(self.base as usize, self.used, capacity, self.page_size, self.capacity)?;
        // SAFETY: a faixa [start, end) está dentro da região, é exclusiva (o cursor avança logo
        // abaixo) e a região nunca é liberada.
        let arena = unsafe { Arena::from_raw(name, self.base.wrapping_add(start), capacity) }?;
        self.used = end;
        REGION_USED.store(end, Ordering::Relaxed);
        Some(arena)
    }

    pub fn thread_memory(&mut self, desc: &ThreadMemoryDesc) -> Option<ThreadMemory> {
        Some(ThreadMemory {
            persistent: self.arena("persistent", desc.persistent)?,
            level: self.arena("level", desc.level)?,
            frame: self.arena("frame", desc.frame)?,
        })
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
        let pages = page_info();
        let page = pages.page_size;
        let mut r = MemoryRegion::reserve(1000).unwrap();
        assert_eq!(r.capacity(), pages.allocation_granularity);
        assert_eq!(r.page_size(), page);

        let desc = ThreadMemoryDesc { persistent: 1000, level: 1, frame: page + 1 };
        let tm = r.thread_memory(&desc).unwrap();
        // cada arena ganha a página inteira; frame ocupa duas
        assert_eq!(tm.persistent.stats().capacity, page);
        assert_eq!(tm.level.stats().capacity, page);
        assert_eq!(tm.frame.stats().capacity, 2 * page);
        assert_eq!(r.used(), 4 * page);

        let s = tm.frame.alloc_slice(2 * page, 0xABu8).unwrap();
        assert!((s.as_ptr() as usize).is_multiple_of(page));
        assert_eq!(s[2 * page - 1], 0xAB);
        assert_eq!(tm.level.used(), 0);

        // não cabe mais que a região
        assert!(r.arena("grande", pages.allocation_granularity).is_none());
    }
}
