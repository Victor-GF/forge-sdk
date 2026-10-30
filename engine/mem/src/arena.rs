//! Arena (bump allocator) de uma thread só.
//!
//! Alocar só empurra um cursor; não existe `free` individual. A memória volta inteira com
//! `reset`. Como `reset` exige `&mut Arena` e as alocações pegam `&Arena`, o compilador prova
//! que nenhuma referência sobrevive ao reset.
//!
//! Só tipos `Copy` entram: a arena nunca roda destrutores.
//!
//! O estado é só o cursor, dentro da própria arena (`Cell`): sem atômicos, sem registro global.
//! Falta de espaço devolve `None`; quem chama decide se é fatal.

use core::cell::Cell;

/// Campos privados: `base`/`capacity` são o invariante de segurança da arena.
pub struct Arena {
    base: *mut u8,
    capacity: usize,
    used: Cell<usize>,
}

// SAFETY: a arena é dona exclusiva da sua faixa de memória, então pode mudar de thread.
// Ela continua !Sync (`Cell` e ponteiro cru): nunca é acessada por duas threads ao mesmo tempo.
unsafe impl Send for Arena {}

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
    /// Cria uma arena sobre memória já existente.
    ///
    /// # Safety
    /// `base` precisa apontar para `capacity` bytes válidos para escrita, usados só por esta
    /// arena e vivos até o fim do programa.
    pub unsafe fn from_raw(base: *mut u8, capacity: usize) -> Arena {
        Arena { base, capacity, used: Cell::new(0) }
    }

    /// Bytes crus, não inicializados. Base para os outros `alloc_*`.
    pub fn alloc_bytes(&self, size: usize, align: usize) -> Option<*mut u8> {
        let (start, end) = bump_reserve(self.base as usize, self.used.get(), size, align, self.capacity)?;
        self.used.set(end);
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

    pub fn reset(&mut self) {
        self.used.set(0);
    }

    pub fn used(&self) -> usize {
        self.used.get()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_arena(capacity: usize) -> Arena {
        let mem = vec![0u8; capacity].leak();
        unsafe { Arena::from_raw(mem.as_mut_ptr(), capacity) }
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
        assert_eq!(a.used(), 40);
    }

    #[test]
    fn out_of_space_returns_none_and_keeps_the_cursor() {
        let a = test_arena(64);
        assert!(a.alloc_slice(16, 0u32).is_some());
        assert!(a.alloc_slice(3, 0u8).is_none());
        assert_eq!(a.used(), 64);
    }

    #[test]
    fn reset_frees_everything() {
        let mut a = test_arena(128);
        let _ = a.alloc_slice(100, 0u8);
        a.reset();
        assert_eq!(a.used(), 0);
        assert!(a.alloc_slice(128, 0u8).is_some());
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
