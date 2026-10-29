//! Arena (bump allocator) de uma thread só.
//!
//! Alocar só empurra um cursor; não existe `free` individual. A memória volta inteira com
//! `arena_reset` (ou até uma marca com `arena_voltar`). Como os dois exigem `&mut Arena` e as
//! alocações pegam `&Arena`, o compilador prova que nenhuma referência sobrevive ao reset.
//!
//! Só tipos `Copy` entram: a arena nunca roda destrutores.

use core::cell::Cell;

/// Campos privados: `base`/`capacidade` são o invariante de segurança da arena.
/// Consulte com `arena_estatistica`.
pub struct Arena {
    base: *mut u8,
    capacidade: usize,
    usado: Cell<usize>,
    pico: Cell<usize>,
    alocacoes: Cell<u32>,
    falhas: Cell<u32>,
    pub nome: &'static str,
}

// SAFETY: a arena é dona exclusiva da sua faixa de memória, então pode mudar de thread.
// Ela continua !Sync (Cell): nunca é acessada por duas threads ao mesmo tempo.
unsafe impl Send for Arena {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArenaEstatistica {
    pub capacidade: usize,
    pub usado: usize,
    /// Maior `usado` já visto (não zera no reset).
    pub pico: usize,
    pub alocacoes: u32,
    /// Pedidos que não couberam.
    pub falhas: u32,
}

/// Reserva `tamanho` bytes alinhados a partir de `usado` num bloco que começa no endereço
/// `base`. Devolve `(inicio, novo_usado)` como deslocamentos, ou `None` se não couber.
/// `alinhamento` precisa ser potência de 2.
pub fn bump_reservar(
    base: usize,
    usado: usize,
    tamanho: usize,
    alinhamento: usize,
    capacidade: usize,
) -> Option<(usize, usize)> {
    if !alinhamento.is_power_of_two() {
        return None;
    }
    let mascara = alinhamento.wrapping_sub(1);
    let endereco = base.checked_add(usado)?;
    let alinhado = endereco.checked_add(mascara)? & !mascara;
    let inicio = alinhado.checked_sub(base)?;
    let fim = inicio.checked_add(tamanho)?;
    if fim > capacidade {
        return None;
    }
    Some((inicio, fim))
}

/// Cria uma arena sobre memória já existente.
///
/// # Safety
/// `base` precisa apontar para `capacidade` bytes válidos para escrita, usados só por esta
/// arena e vivos até o fim do programa.
pub unsafe fn arena_de_bytes(nome: &'static str, base: *mut u8, capacidade: usize) -> Arena {
    Arena {
        base,
        capacidade,
        usado: Cell::new(0),
        pico: Cell::new(0),
        alocacoes: Cell::new(0),
        falhas: Cell::new(0),
        nome,
    }
}

/// Bytes crus, não inicializados. Base para os outros `arena_*`.
pub fn arena_bytes(a: &Arena, tamanho: usize, alinhamento: usize) -> Option<*mut u8> {
    let Some((inicio, fim)) =
        bump_reservar(a.base as usize, a.usado.get(), tamanho, alinhamento, a.capacidade)
    else {
        a.falhas.set(a.falhas.get().saturating_add(1));
        return None;
    };
    a.usado.set(fim);
    if fim > a.pico.get() {
        a.pico.set(fim);
    }
    a.alocacoes.set(a.alocacoes.get().saturating_add(1));
    Some(a.base.wrapping_add(inicio))
}

/// `n` elementos, todos iniciados com `valor`.
// Cada chamada devolve uma faixa nova e disjunta; reset exige `&mut`, então não há aliasing.
#[allow(clippy::mut_from_ref)]
pub fn arena_slice<T: Copy>(a: &Arena, n: usize, valor: T) -> Option<&mut [T]> {
    let tamanho = size_of::<T>().checked_mul(n)?;
    let p = arena_bytes(a, tamanho, align_of::<T>())?.cast::<T>();
    for i in 0..n {
        // SAFETY: `p` tem espaço alinhado para `n` elementos de `T`.
        unsafe { p.add(i).write(valor) };
    }
    // SAFETY: os `n` elementos acabaram de ser iniciados e a faixa é exclusiva.
    Some(unsafe { core::slice::from_raw_parts_mut(p, n) })
}

/// Um valor de `T` na arena. Para tabelas grandes use `arena_slice` (evita montar o valor
/// inteiro na stack antes de copiar).
#[allow(clippy::mut_from_ref)]
pub fn arena_valor<T: Copy>(a: &Arena, valor: T) -> Option<&mut T> {
    let p = arena_bytes(a, size_of::<T>(), align_of::<T>())?.cast::<T>();
    // SAFETY: `p` tem espaço alinhado para um `T`, exclusivo desta chamada.
    unsafe {
        p.write(valor);
        Some(&mut *p)
    }
}

pub fn arena_reset(a: &mut Arena) {
    a.usado.set(0);
}

/// Posição atual, para voltar depois com `arena_voltar` (memória temporária em escopo).
pub fn arena_marca(a: &Arena) -> usize {
    a.usado.get()
}

/// Libera tudo que foi alocado depois de `marca`. Marca à frente do cursor é ignorada.
pub fn arena_voltar(a: &mut Arena, marca: usize) {
    if marca <= a.usado.get() {
        a.usado.set(marca);
    }
}

pub fn arena_usado(a: &Arena) -> usize {
    a.usado.get()
}

pub fn arena_estatistica(a: &Arena) -> ArenaEstatistica {
    ArenaEstatistica {
        capacidade: a.capacidade,
        usado: a.usado.get(),
        pico: a.pico.get(),
        alocacoes: a.alocacoes.get(),
        falhas: a.falhas.get(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena_teste(capacidade: usize) -> Arena {
        let mem = vec![0u8; capacidade].leak();
        unsafe { arena_de_bytes("teste", mem.as_mut_ptr(), capacidade) }
    }

    #[test]
    fn bump_alinha_e_respeita_capacidade() {
        assert_eq!(bump_reservar(1000, 0, 8, 16, 64), Some((8, 16)));
        assert_eq!(bump_reservar(1024, 3, 4, 4, 64), Some((4, 8)));
        assert_eq!(bump_reservar(1024, 60, 8, 1, 64), None);
        assert_eq!(bump_reservar(1024, 0, 8, 3, 64), None);
        assert_eq!(bump_reservar(usize::MAX, 1, 1, 1, 64), None);
    }

    #[test]
    fn slice_alinhado_e_iniciado() {
        let a = arena_teste(256);
        let _ = arena_slice(&a, 3, 1u8).unwrap();
        let s = arena_slice(&a, 4, 7u64).unwrap();
        assert!((s.as_ptr() as usize).is_multiple_of(align_of::<u64>()));
        assert_eq!(s, &[7, 7, 7, 7]);
        s[1] = 9;
        assert_eq!(arena_estatistica(&a).alocacoes, 2);
    }

    #[test]
    fn falta_de_espaco_devolve_none_e_conta() {
        let a = arena_teste(64);
        assert!(arena_slice(&a, 16, 0u32).is_some());
        assert!(arena_slice(&a, 1, 0u8).is_none());
        let e = arena_estatistica(&a);
        assert_eq!((e.usado, e.falhas), (64, 1));
    }

    #[test]
    fn reset_e_marca() {
        let mut a = arena_teste(128);
        let _ = arena_slice(&a, 10, 0u8);
        let m = arena_marca(&a);
        let _ = arena_slice(&a, 50, 0u8);
        arena_voltar(&mut a, m);
        assert_eq!(arena_usado(&a), 10);
        arena_reset(&mut a);
        assert_eq!(arena_usado(&a), 0);
        assert_eq!(arena_estatistica(&a).pico, 60);
    }

    #[test]
    fn arena_vai_para_outra_thread() {
        let a = arena_teste(64);
        let usado = std::thread::spawn(move || {
            let _ = arena_valor(&a, 5u32);
            arena_usado(&a)
        })
        .join()
        .unwrap();
        assert_eq!(usado, 4);
    }
}
