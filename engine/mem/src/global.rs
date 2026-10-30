//! Heap geral: o alocador global do Rust (`#[global_allocator]`).
//!
//! Os dados da engine e do jogo NÃO passam por aqui: eles saem das arenas. O heap geral atende
//! o que não controlamos (std, dependências).
//!
//! Só repassa ao heap do sistema (`std::alloc::System`: `HeapAlloc` no Windows, `malloc` da
//! glibc no Linux) e conta alocações e bytes vivos, para medir a meta do jogo: zero alocações
//! por frame. Thread-safe porque qualquer thread pode alocar pela std: os contadores são
//! atômicos.
//!
//! Uso no binário (fora de testes, que usam o alocador do sistema direto):
//! ```ignore
//! #[cfg(not(test))]
//! #[global_allocator]
//! static ALLOCATOR: forja_mem::global::GlobalAllocator = forja_mem::global::GlobalAllocator;
//! ```

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::alloc::System;

/// `impl` de trait exigida pela linguagem para o alocador global.
pub struct GlobalAllocator;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeapStats {
    /// Alocações desde o início do processo (`realloc` conta como uma).
    pub allocations: u64,
    pub live_bytes: usize,
}

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);

pub fn stats() -> HeapStats {
    HeapStats { allocations: ALLOCATIONS.load(Ordering::Relaxed), live_bytes: LIVE.load(Ordering::Relaxed) }
}

// SAFETY: só repassa ao `System`, com os mesmos argumentos; os contadores não tocam a memória.
unsafe impl GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: repassa o contrato de `GlobalAlloc::alloc`.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: repassa o contrato de `GlobalAlloc::alloc_zeroed`.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: repassa o contrato de `GlobalAlloc::dealloc`.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: repassa o contrato de `GlobalAlloc::realloc`.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            LIVE.fetch_add(new_size, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // testes seguem as mesmas exceções do clippy.toml
mod tests {
    use super::*;

    // Chama o alocador direto (nos testes ele não é o global): só este teste mexe nos contadores.
    #[test]
    fn counts_allocations_and_live_bytes() {
        let before = stats();
        let l = Layout::from_size_align(24, 8).unwrap();
        let p = unsafe { GlobalAllocator.alloc_zeroed(l) };
        assert!(!p.is_null());
        assert!((0..24).all(|i| unsafe { *p.add(i) } == 0));
        assert_eq!(stats(), HeapStats { allocations: before.allocations + 1, live_bytes: before.live_bytes + 24 });

        let p = unsafe { GlobalAllocator.realloc(p, l, 100) };
        assert!(!p.is_null());
        assert_eq!(stats(), HeapStats { allocations: before.allocations + 2, live_bytes: before.live_bytes + 100 });

        unsafe { GlobalAllocator.dealloc(p, Layout::from_size_align(100, 8).unwrap()) };
        assert_eq!(stats().live_bytes, before.live_bytes);
    }
}
