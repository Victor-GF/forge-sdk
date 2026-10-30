//! Arena (bump allocator) de uma thread só.
//!
//! Alocar só empurra um cursor; não existe `free` individual. A memória volta inteira com
//! `reset` (ou até uma marca com `rewind`). Como os dois exigem `&mut Arena` e as alocações
//! pegam `&Arena`, o compilador prova que nenhuma referência sobrevive ao reset.
//!
//! Só tipos `Copy` entram: a arena nunca roda destrutores.
//!
//! Estatísticas ficam em `stats`, legíveis de outras threads pelo crash report.
//! Ao passar de 90% da capacidade pela primeira vez, a arena registra um aviso no log.

use core::sync::atomic::Ordering;

use forja_diag::{crash, log};

use crate::stats::{ArenaSlot, register_arena};

/// Campos privados: `base`/`capacity` são o invariante de segurança da arena.
/// Consulte com `stats()`.
pub struct Arena {
    base: *mut u8,
    capacity: usize,
    /// Acima disso (90%), avisa no log uma vez.
    warn_threshold: usize,
    slot: &'static ArenaSlot,
    name: &'static str,
}

// SAFETY: a arena é dona exclusiva da sua faixa de memória, então pode mudar de thread.
// Ela continua !Sync (ponteiro cru): nunca é acessada por duas threads ao mesmo tempo.
unsafe impl Send for Arena {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArenaStats {
    pub capacity: usize,
    pub used: usize,
    /// Maior `used` já visto (não zera no reset).
    pub peak: usize,
    pub allocations: u32,
    /// Pedidos que não couberam.
    pub failures: u32,
    /// Tamanho do último pedido que não coube.
    pub last_failed_size: usize,
}

/// Posição do cursor, obtida com `Arena::mark` e usada em `Arena::rewind`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark(usize);

/// Reserva `size` bytes alinhados a partir de `used` num bloco que começa no endereço
/// `base`. Devolve `(start, new_used)` como deslocamentos, ou `None` se não couber.
/// `align` precisa ser potência de 2.
pub(crate) fn bump_reserve(
    base: usize,
    used: usize,
    size: usize,
    align: usize,
    capacity: usize,
) -> Option<(usize, usize)> {
    if !align.is_power_of_two() {
        return None;
    }
    let mask = align.wrapping_sub(1);
    let address = base.checked_add(used)?;
    let aligned = address.checked_add(mask)? & !mask;
    let start = aligned.checked_sub(base)?;
    let end = start.checked_add(size)?;
    if end > capacity {
        return None;
    }
    Some((start, end))
}

impl Arena {
    /// Cria uma arena sobre memória já existente. `None` se o registro de estatísticas estiver
    /// cheio (`stats::ARENAS_MAX` arenas).
    ///
    /// # Safety
    /// `base` precisa apontar para `capacity` bytes válidos para escrita, usados só por esta
    /// arena e vivos até o fim do programa.
    pub unsafe fn from_raw(name: &'static str, base: *mut u8, capacity: usize) -> Option<Arena> {
        let slot = register_arena(name, capacity)?;
        let warn_threshold = capacity.saturating_div(10).saturating_mul(9);
        Some(Arena { base, capacity, warn_threshold, slot, name })
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Bytes crus, não inicializados. Base para os outros `alloc_*`.
    pub fn alloc_bytes(&self, size: usize, align: usize) -> Option<*mut u8> {
        let s = self.slot;
        let used = s.used.load(Ordering::Relaxed);
        let Some((start, end)) = bump_reserve(self.base as usize, used, size, align, self.capacity) else {
            s.failures.store(s.failures.load(Ordering::Relaxed).saturating_add(1), Ordering::Relaxed);
            s.last_failed_size.store(size, Ordering::Relaxed);
            return None;
        };
        s.used.store(end, Ordering::Relaxed);
        s.allocations.store(s.allocations.load(Ordering::Relaxed).saturating_add(1), Ordering::Relaxed);
        if end > s.peak.load(Ordering::Relaxed) {
            s.peak.store(end, Ordering::Relaxed);
            if end > self.warn_threshold && !s.warned.load(Ordering::Relaxed) {
                s.warned.store(true, Ordering::Relaxed);
                log::warn(format_args!(
                    "arena '{}' passou de 90% do orçamento: {} de {} bytes",
                    self.name, end, self.capacity
                ));
            }
        }
        Some(self.base.wrapping_add(start))
    }

    /// `n` elementos, todos iniciados com `value`.
    // Cada chamada devolve uma faixa nova e disjunta; reset exige `&mut`, então não há aliasing.
    #[allow(clippy::mut_from_ref)]
    pub fn alloc_slice<T: Copy>(&self, n: usize, value: T) -> Option<&mut [T]> {
        let size = size_of::<T>().checked_mul(n)?;
        let p = self.alloc_bytes(size, align_of::<T>())?.cast::<T>();
        for i in 0..n {
            // SAFETY: `p` tem espaço alinhado para `n` elementos de `T`.
            unsafe { p.add(i).write(value) };
        }
        // SAFETY: os `n` elementos acabaram de ser iniciados e a faixa é exclusiva.
        Some(unsafe { core::slice::from_raw_parts_mut(p, n) })
    }

    /// Um valor de `T` na arena. Para tabelas grandes use `alloc_slice` (evita montar o valor
    /// inteiro na stack antes de copiar).
    #[allow(clippy::mut_from_ref)]
    pub fn alloc_value<T: Copy>(&self, value: T) -> Option<&mut T> {
        let p = self.alloc_bytes(size_of::<T>(), align_of::<T>())?.cast::<T>();
        // SAFETY: `p` tem espaço alinhado para um `T`, exclusivo desta chamada.
        unsafe {
            p.write(value);
            Some(&mut *p)
        }
    }

    /// Para quando a falta de espaço é fatal: grava o crash report e aborta. O pedido que
    /// falhou e o uso da arena vão para o stderr (texto livre não entra no relatório).
    ///
    /// `let Some(x) = arena.alloc_slice(n, 0) else { arena.oom_fatal() };`
    pub fn oom_fatal(&self) -> ! {
        let s = self.stats();
        crash::fatal(
            "OOM de arena",
            format_args!(
                "arena '{}' sem espaço: pedido de {} bytes com {} de {} usados (pico {}). \
                 Aumente o orçamento da arena ou reduza o uso.",
                self.name, s.last_failed_size, s.used, s.capacity, s.peak
            ),
        )
    }

    pub fn reset(&mut self) {
        self.slot.used.store(0, Ordering::Relaxed);
    }

    /// Posição atual, para voltar depois com `rewind` (memória temporária em escopo).
    pub fn mark(&self) -> Mark {
        Mark(self.slot.used.load(Ordering::Relaxed))
    }

    /// Libera tudo que foi alocado depois de `mark`. Marca à frente do cursor é ignorada.
    pub fn rewind(&mut self, mark: Mark) {
        if mark.0 <= self.slot.used.load(Ordering::Relaxed) {
            self.slot.used.store(mark.0, Ordering::Relaxed);
        }
    }

    pub fn used(&self) -> usize {
        self.slot.used.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> ArenaStats {
        let s = self.slot;
        ArenaStats {
            capacity: self.capacity,
            used: s.used.load(Ordering::Relaxed),
            peak: s.peak.load(Ordering::Relaxed),
            allocations: s.allocations.load(Ordering::Relaxed),
            failures: s.failures.load(Ordering::Relaxed),
            last_failed_size: s.last_failed_size.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_arena(capacity: usize) -> Arena {
        let mem = vec![0u8; capacity].leak();
        unsafe { Arena::from_raw("teste", mem.as_mut_ptr(), capacity) }.unwrap()
    }

    #[test]
    fn bump_aligns_and_respects_capacity() {
        assert_eq!(bump_reserve(1000, 0, 8, 16, 64), Some((8, 16)));
        assert_eq!(bump_reserve(1024, 3, 4, 4, 64), Some((4, 8)));
        assert_eq!(bump_reserve(1024, 60, 8, 1, 64), None);
        assert_eq!(bump_reserve(1024, 0, 8, 3, 64), None);
        assert_eq!(bump_reserve(usize::MAX, 1, 1, 1, 64), None);
    }

    #[test]
    fn slice_is_aligned_and_initialized() {
        let a = test_arena(256);
        let _ = a.alloc_slice(3, 1u8).unwrap();
        let s = a.alloc_slice(4, 7u64).unwrap();
        assert!((s.as_ptr() as usize).is_multiple_of(align_of::<u64>()));
        assert_eq!(s, &[7, 7, 7, 7]);
        s[1] = 9;
        assert_eq!(a.stats().allocations, 2);
    }

    #[test]
    fn out_of_space_returns_none_and_counts() {
        let a = test_arena(64);
        assert!(a.alloc_slice(16, 0u32).is_some());
        assert!(a.alloc_slice(3, 0u8).is_none());
        let s = a.stats();
        assert_eq!((s.used, s.failures, s.last_failed_size), (64, 1, 3));
    }

    #[test]
    fn reset_and_rewind() {
        let mut a = test_arena(128);
        let _ = a.alloc_slice(10, 0u8);
        let m = a.mark();
        let _ = a.alloc_slice(50, 0u8);
        a.rewind(m);
        assert_eq!(a.used(), 10);
        a.reset();
        assert_eq!(a.used(), 0);
        assert_eq!(a.stats().peak, 60);
    }

    #[test]
    fn arena_moves_to_another_thread() {
        let a = test_arena(64);
        let used = std::thread::spawn(move || {
            let _ = a.alloc_value(5u32);
            a.used()
        })
        .join()
        .unwrap();
        assert_eq!(used, 4);
    }
}
