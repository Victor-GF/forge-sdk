//! Alocador global do Rust (`#[global_allocator]`).
//!
//! Não chama o SO: atende pedidos com bump num buffer fixo em `.bss`. Existe só para a
//! inicialização da std (antes e logo no começo do `main`). Depois que o jogo chama
//! `global_travar`, qualquer alocação — `Vec`, `String`, `Box`, `format!`, de qualquer
//! crate — imprime um aviso e aborta. Rodando no debugger, o abort para na pilha culpada.
//!
//! Liberar só devolve espaço se for o último bloco (LIFO); o resto fica perdido até o fim.
//! Como o uso é só na inicialização, isso é proposital.
//!
//! Uso no binário (fora de testes, que precisam do alocador do sistema):
//! ```ignore
//! #[cfg(not(test))]
//! #[global_allocator]
//! static ALOCADOR: forja_mem::global::AlocadorGlobal = forja_mem::global::AlocadorGlobal;
//! ```

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::fmt::Write;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use forja_core::texto::{texto_bytes, texto_novo};

use crate::arena::bump_reservar;

pub const GLOBAL_CAPACIDADE: usize = 256 * 1024;

#[repr(C, align(64))]
struct BufGlobal(UnsafeCell<[u8; GLOBAL_CAPACIDADE]>);

// SAFETY: o buffer só é acessado por faixas entregues pelo bump atômico; cada byte tem no
// máximo um dono por vez.
unsafe impl Sync for BufGlobal {}

static GLOBAL_BUF: BufGlobal = BufGlobal(UnsafeCell::new([0; GLOBAL_CAPACIDADE]));
static GLOBAL_USADO: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_ALOCACOES: AtomicU64 = AtomicU64::new(0);
static GLOBAL_LIBERACOES: AtomicU64 = AtomicU64::new(0);
static GLOBAL_TRAVADO: AtomicBool = AtomicBool::new(false);

/// Única `impl` de trait da engine fora de testes: exigida pela linguagem para o alocador global.
pub struct AlocadorGlobal;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalEstatistica {
    pub capacidade: usize,
    pub usado: usize,
    pub alocacoes: u64,
    pub liberacoes: u64,
    pub travado: bool,
}

/// A partir daqui, toda alocação pelo alocador global aborta o programa.
pub fn global_travar() {
    GLOBAL_TRAVADO.store(true, Ordering::Release);
}

/// Volta a permitir alocações (ex.: em volta de uma biblioteca que aloca). Use em escopo curto
/// e explícito; o que for alocado aqui sai do buffer fixo e quase nunca volta.
pub fn global_destravar() {
    GLOBAL_TRAVADO.store(false, Ordering::Release);
}

pub fn global_estatistica() -> GlobalEstatistica {
    GlobalEstatistica {
        capacidade: GLOBAL_CAPACIDADE,
        usado: GLOBAL_USADO.load(Ordering::Relaxed),
        alocacoes: GLOBAL_ALOCACOES.load(Ordering::Relaxed),
        liberacoes: GLOBAL_LIBERACOES.load(Ordering::Relaxed),
        travado: GLOBAL_TRAVADO.load(Ordering::Relaxed),
    }
}

fn global_violacao(tamanho: usize) -> ! {
    let mut buf = [0u8; 192];
    let mut t = texto_novo(&mut buf);
    let _ = writeln!(
        t,
        "[forja_mem] alocacao de {tamanho} bytes com o alocador global travado. \
         Rode no debugger para ver a pilha."
    );
    let _ = std::io::Write::write_all(&mut std::io::stderr(), texto_bytes(&t));
    std::process::abort()
}

// SAFETY: devolve faixas alinhadas e disjuntas do buffer estático, ou nulo quando não cabe.
unsafe impl GlobalAlloc for AlocadorGlobal {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if GLOBAL_TRAVADO.load(Ordering::Acquire) {
            global_violacao(layout.size());
        }
        let base = GLOBAL_BUF.0.get().cast::<u8>();
        let mut usado = GLOBAL_USADO.load(Ordering::Relaxed);
        loop {
            let Some((inicio, fim)) =
                bump_reservar(base as usize, usado, layout.size(), layout.align(), GLOBAL_CAPACIDADE)
            else {
                return null_mut();
            };
            match GLOBAL_USADO.compare_exchange_weak(usado, fim, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => {
                    GLOBAL_ALOCACOES.fetch_add(1, Ordering::Relaxed);
                    return base.wrapping_add(inicio);
                }
                Err(atual) => usado = atual,
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        GLOBAL_LIBERACOES.fetch_add(1, Ordering::Relaxed);
        let inicio = (ptr as usize).wrapping_sub(GLOBAL_BUF.0.get() as usize);
        if let Some(fim) = inicio.checked_add(layout.size()) {
            // Se era o último bloco, devolve o espaço; senão não faz nada.
            let _ = GLOBAL_USADO.compare_exchange(fim, inicio, Ordering::AcqRel, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Chama o alocador direto (nos testes ele não é o global). Não testa o travamento,
    // porque abortaria o processo de testes.
    #[test]
    fn bump_alinhado_e_devolve_ultimo_bloco() {
        let l = Layout::from_size_align(32, 16).unwrap();
        unsafe {
            let a = AlocadorGlobal.alloc(l);
            assert!(!a.is_null());
            assert!((a as usize).is_multiple_of(16));
            let antes = global_estatistica().usado;
            let b = AlocadorGlobal.alloc(l);
            AlocadorGlobal.dealloc(b, l);
            assert_eq!(global_estatistica().usado, antes);
            AlocadorGlobal.dealloc(a, l);
        }
    }

    #[test]
    fn pedido_maior_que_o_buffer_devolve_nulo() {
        let l = Layout::from_size_align(GLOBAL_CAPACIDADE + 1, 8).unwrap();
        assert!(unsafe { AlocadorGlobal.alloc(l) }.is_null());
    }
}
