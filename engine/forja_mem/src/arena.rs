//! Arena (bump allocator) de uma thread só.
//!
//! Alocar só empurra um cursor; não existe `free` individual. A memória volta inteira com
//! `arena_reset` (ou até uma marca com `arena_voltar`). Como os dois exigem `&mut Arena` e as
//! alocações pegam `&Arena`, o compilador prova que nenhuma referência sobrevive ao reset.
//!
//! Só tipos `Copy` entram: a arena nunca roda destrutores.
//!
//! Estatísticas ficam no painel (`painel.rs`), legíveis de outras threads pelo crash report.
//! Ao passar de 90% da capacidade pela primeira vez, a arena registra um aviso no log.

use core::sync::atomic::Ordering;

use forja_diag::crash::diag_fatal;
use forja_diag::log::log_aviso;

use crate::painel::{PainelSlot, painel_registrar};

/// Campos privados: `base`/`capacidade` são o invariante de segurança da arena.
/// Consulte com `arena_estatistica`.
pub struct Arena {
    base: *mut u8,
    capacidade: usize,
    /// Acima disso (90%), avisa no log uma vez.
    limite_aviso: usize,
    painel: &'static PainelSlot,
    pub nome: &'static str,
}

// SAFETY: a arena é dona exclusiva da sua faixa de memória, então pode mudar de thread.
// Ela continua !Sync (ponteiro cru): nunca é acessada por duas threads ao mesmo tempo.
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
    /// Tamanho do último pedido que não coube.
    pub ultima_falha: usize,
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

/// Cria uma arena sobre memória já existente. `None` se o painel estiver cheio
/// (`painel::PAINEL_MAX` arenas).
///
/// # Safety
/// `base` precisa apontar para `capacidade` bytes válidos para escrita, usados só por esta
/// arena e vivos até o fim do programa.
pub unsafe fn arena_de_bytes(nome: &'static str, base: *mut u8, capacidade: usize) -> Option<Arena> {
    let painel = painel_registrar(nome, capacidade)?;
    let limite_aviso = capacidade.saturating_div(10).saturating_mul(9);
    Some(Arena { base, capacidade, limite_aviso, painel, nome })
}

/// Bytes crus, não inicializados. Base para os outros `arena_*`.
pub fn arena_bytes(a: &Arena, tamanho: usize, alinhamento: usize) -> Option<*mut u8> {
    let p = a.painel;
    let usado = p.usado.load(Ordering::Relaxed);
    let Some((inicio, fim)) = bump_reservar(a.base as usize, usado, tamanho, alinhamento, a.capacidade)
    else {
        p.falhas.store(p.falhas.load(Ordering::Relaxed).saturating_add(1), Ordering::Relaxed);
        p.ultima_falha.store(tamanho, Ordering::Relaxed);
        return None;
    };
    p.usado.store(fim, Ordering::Relaxed);
    p.alocacoes.store(p.alocacoes.load(Ordering::Relaxed).saturating_add(1), Ordering::Relaxed);
    if fim > p.pico.load(Ordering::Relaxed) {
        p.pico.store(fim, Ordering::Relaxed);
        if fim > a.limite_aviso && !p.avisou.load(Ordering::Relaxed) {
            p.avisou.store(true, Ordering::Relaxed);
            log_aviso(format_args!(
                "arena '{}' passou de 90% do orçamento: {} de {} bytes",
                a.nome, fim, a.capacidade
            ));
        }
    }
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

/// Para quando a falta de espaço é fatal: grava o crash report (com o pedido que falhou e o
/// uso da arena) e aborta.
///
/// `let Some(x) = arena_slice(&a, n, 0) else { arena_oom_fatal(&a) };`
pub fn arena_oom_fatal(a: &Arena) -> ! {
    let e = arena_estatistica(a);
    diag_fatal(
        "OOM de arena",
        format_args!(
            "arena '{}' sem espaço: pedido de {} bytes com {} de {} usados (pico {}). \
             Aumente o orçamento da arena ou reduza o uso.",
            a.nome, e.ultima_falha, e.usado, e.capacidade, e.pico
        ),
    )
}

pub fn arena_reset(a: &mut Arena) {
    a.painel.usado.store(0, Ordering::Relaxed);
}

/// Posição atual, para voltar depois com `arena_voltar` (memória temporária em escopo).
pub fn arena_marca(a: &Arena) -> usize {
    a.painel.usado.load(Ordering::Relaxed)
}

/// Libera tudo que foi alocado depois de `marca`. Marca à frente do cursor é ignorada.
pub fn arena_voltar(a: &mut Arena, marca: usize) {
    if marca <= a.painel.usado.load(Ordering::Relaxed) {
        a.painel.usado.store(marca, Ordering::Relaxed);
    }
}

pub fn arena_usado(a: &Arena) -> usize {
    a.painel.usado.load(Ordering::Relaxed)
}

pub fn arena_estatistica(a: &Arena) -> ArenaEstatistica {
    let p = a.painel;
    ArenaEstatistica {
        capacidade: a.capacidade,
        usado: p.usado.load(Ordering::Relaxed),
        pico: p.pico.load(Ordering::Relaxed),
        alocacoes: p.alocacoes.load(Ordering::Relaxed),
        falhas: p.falhas.load(Ordering::Relaxed),
        ultima_falha: p.ultima_falha.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena_teste(capacidade: usize) -> Arena {
        let mem = vec![0u8; capacidade].leak();
        unsafe { arena_de_bytes("teste", mem.as_mut_ptr(), capacidade) }.unwrap()
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
        assert!(arena_slice(&a, 3, 0u8).is_none());
        let e = arena_estatistica(&a);
        assert_eq!((e.usado, e.falhas, e.ultima_falha), (64, 1, 3));
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
