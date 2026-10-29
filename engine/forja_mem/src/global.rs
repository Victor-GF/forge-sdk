//! Alocador global do Rust (`#[global_allocator]`).
//!
//! Não chama o SO: atende pedidos com bump num buffer fixo em `.bss`. Existe só para a
//! inicialização da std (antes e logo no começo do `main`). Depois que o jogo chama
//! `global::lock()`, qualquer alocação — `Vec`, `String`, `Box`, `format!`, de qualquer
//! crate — grava um crash report (com a pilha de quem alocou) e aborta.
//!
//! Exceção: durante um panic a alocação é permitida, porque a std formata a mensagem num
//! `String` antes de chamar o hook de panic; assim o relatório sai com a mensagem certa.
//!
//! Buffer esgotado também é fatal (com relatório), em vez de devolver nulo: o handler de
//! erro de alocação da std aborta sem passar pelo crash report.
//!
//! Liberar só devolve espaço se for o último bloco (LIFO); o resto fica perdido até o fim.
//! Como o uso é só na inicialização, isso é proposital.
//!
//! Uso no binário (fora de testes, que precisam do alocador do sistema):
//! ```ignore
//! #[cfg(not(test))]
//! #[global_allocator]
//! static ALLOCATOR: forja_mem::global::GlobalAllocator = forja_mem::global::GlobalAllocator;
//! ```

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use forja_diag::crash;

use crate::arena::bump_reserve;

pub const GLOBAL_CAPACITY: usize = 256 * 1024;

#[repr(C, align(64))]
struct GlobalBuf(UnsafeCell<[u8; GLOBAL_CAPACITY]>);

// SAFETY: o buffer só é acessado por faixas entregues pelo bump atômico; cada byte tem no
// máximo um dono por vez.
unsafe impl Sync for GlobalBuf {}

static BUF: GlobalBuf = GlobalBuf(UnsafeCell::new([0; GLOBAL_CAPACITY]));
static USED: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
static LOCKED: AtomicBool = AtomicBool::new(false);

/// `impl` de trait exigida pela linguagem para o alocador global.
pub struct GlobalAllocator;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalStats {
    pub capacity: usize,
    pub used: usize,
    pub allocations: u64,
    pub frees: u64,
    pub locked: bool,
}

/// A partir daqui, toda alocação pelo alocador global é fatal.
pub fn lock() {
    LOCKED.store(true, Ordering::Release);
}

/// Volta a permitir alocações (ex.: em volta de uma biblioteca que aloca). Use em escopo curto
/// e explícito; o que for alocado aqui sai do buffer fixo e quase nunca volta.
pub fn unlock() {
    LOCKED.store(false, Ordering::Release);
}

pub fn stats() -> GlobalStats {
    GlobalStats {
        capacity: GLOBAL_CAPACITY,
        used: USED.load(Ordering::Relaxed),
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        frees: FREES.load(Ordering::Relaxed),
        locked: LOCKED.load(Ordering::Relaxed),
    }
}

// SAFETY: devolve faixas alinhadas e disjuntas do buffer estático; quando não cabe, não
// devolve (crash report + abort).
unsafe impl GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if LOCKED.load(Ordering::Acquire) && !std::thread::panicking() {
            crash::fatal(
                "alocação com o alocador global travado",
                format_args!(
                    "pedido de {} bytes (alinhamento {}) depois de global::lock(). \
                     A pilha abaixo mostra quem alocou (Vec, String, Box, format!...).",
                    layout.size(),
                    layout.align()
                ),
            );
        }
        let base = BUF.0.get().cast::<u8>();
        let mut used = USED.load(Ordering::Relaxed);
        loop {
            let Some((start, end)) =
                bump_reserve(base as usize, used, layout.size(), layout.align(), GLOBAL_CAPACITY)
            else {
                crash::fatal(
                    "alocador global esgotado",
                    format_args!(
                        "pedido de {} bytes com {} de {} usados. A inicialização da std ou de \
                         uma dependência está alocando mais que o previsto.",
                        layout.size(),
                        used,
                        GLOBAL_CAPACITY
                    ),
                );
            };
            match USED.compare_exchange_weak(used, end, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => {
                    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                    return base.wrapping_add(start);
                }
                Err(current) => used = current,
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREES.fetch_add(1, Ordering::Relaxed);
        let start = (ptr as usize).wrapping_sub(BUF.0.get() as usize);
        if let Some(end) = start.checked_add(layout.size()) {
            // Se era o último bloco, devolve o espaço; senão não faz nada.
            let _ = USED.compare_exchange(end, start, Ordering::AcqRel, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Chama o alocador direto (nos testes ele não é o global). Travamento e esgotamento
    // abortam o processo: são testados em `tests/crash`.
    #[test]
    fn bump_is_aligned_and_frees_last_block() {
        let l = Layout::from_size_align(32, 16).unwrap();
        unsafe {
            let a = GlobalAllocator.alloc(l);
            assert!(!a.is_null());
            assert!((a as usize).is_multiple_of(16));
            let before = stats().used;
            let b = GlobalAllocator.alloc(l);
            GlobalAllocator.dealloc(b, l);
            assert_eq!(stats().used, before);
            GlobalAllocator.dealloc(a, l);
        }
    }
}
