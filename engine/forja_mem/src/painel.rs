//! Painel de memória: estatísticas de todas as arenas, legíveis de qualquer thread.
//!
//! Cada arena tem um slot com um único escritor (a thread dona); os campos são atômicos
//! para que o crash report (e, no futuro, o inspetor) leia de outra thread sem corrida.
//! No x86_64, load/store `Relaxed` são `mov` comuns: o custo por alocação é o mesmo de antes.
//!
//! Slots nunca são liberados (arenas vivem até o fim do programa).

use core::cell::UnsafeCell;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use forja_core::texto::Texto;

use crate::global::global_estatistica;

pub const PAINEL_MAX: usize = 64;

pub(crate) struct PainelSlot {
    ativo: AtomicBool,
    nome: UnsafeCell<&'static str>,
    pub(crate) capacidade: AtomicUsize,
    pub(crate) usado: AtomicUsize,
    pub(crate) pico: AtomicUsize,
    pub(crate) alocacoes: AtomicU32,
    pub(crate) falhas: AtomicU32,
    /// Tamanho do último pedido que não coube.
    pub(crate) ultima_falha: AtomicUsize,
    pub(crate) avisou: AtomicBool,
}

// SAFETY: `nome` é escrito uma vez, com a trava do painel, antes de `ativo` virar `true`
// (Release); leitores só o leem depois de ver `ativo` (Acquire).
unsafe impl Sync for PainelSlot {}

const fn slot_vazio() -> PainelSlot {
    PainelSlot {
        ativo: AtomicBool::new(false),
        nome: UnsafeCell::new(""),
        capacidade: AtomicUsize::new(0),
        usado: AtomicUsize::new(0),
        pico: AtomicUsize::new(0),
        alocacoes: AtomicU32::new(0),
        falhas: AtomicU32::new(0),
        ultima_falha: AtomicUsize::new(0),
        avisou: AtomicBool::new(false),
    }
}

static SLOTS: [PainelSlot; PAINEL_MAX] = [const { slot_vazio() }; PAINEL_MAX];
static SLOTS_N: AtomicUsize = AtomicUsize::new(0);
static SLOTS_TRAVA: AtomicBool = AtomicBool::new(false);

// Região do programa (memoria_init), para o relatório.
pub(crate) static REGIAO_CAPACIDADE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REGIAO_USADO: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REGIAO_PAGINA: AtomicUsize = AtomicUsize::new(0);

/// Leitura de um slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PainelLeitura {
    pub nome: &'static str,
    pub capacidade: usize,
    pub usado: usize,
    pub pico: usize,
    pub alocacoes: u32,
    pub falhas: u32,
    pub ultima_falha: usize,
}

/// `None` quando os `PAINEL_MAX` slots já foram usados.
pub(crate) fn painel_registrar(nome: &'static str, capacidade: usize) -> Option<&'static PainelSlot> {
    while SLOTS_TRAVA
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let i = SLOTS_N.load(Ordering::Relaxed);
    let slot = SLOTS.get(i);
    if let Some(s) = slot {
        // SAFETY: trava pega e o slot ainda não está ativo (ninguém lê `nome`).
        unsafe { *s.nome.get() = nome };
        s.capacidade.store(capacidade, Ordering::Relaxed);
        s.ativo.store(true, Ordering::Release);
        SLOTS_N.store(i.saturating_add(1), Ordering::Release);
    }
    SLOTS_TRAVA.store(false, Ordering::Release);
    slot
}

pub fn painel_quantidade() -> usize {
    SLOTS_N.load(Ordering::Acquire)
}

pub fn painel_ler(i: usize) -> Option<PainelLeitura> {
    let s = SLOTS.get(i)?;
    if !s.ativo.load(Ordering::Acquire) {
        return None;
    }
    Some(PainelLeitura {
        // SAFETY: `ativo` visto como `true`; `nome` não muda mais.
        nome: unsafe { *s.nome.get() },
        capacidade: s.capacidade.load(Ordering::Relaxed),
        usado: s.usado.load(Ordering::Relaxed),
        pico: s.pico.load(Ordering::Relaxed),
        alocacoes: s.alocacoes.load(Ordering::Relaxed),
        falhas: s.falhas.load(Ordering::Relaxed),
        ultima_falha: s.ultima_falha.load(Ordering::Relaxed),
    })
}

fn porcento(parte: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    parte as f64 * 100.0 / total as f64
}

/// Seção "memória da forja" do crash report.
pub(crate) fn mem_secao_crash(t: &mut Texto) {
    let cap = REGIAO_CAPACIDADE.load(Ordering::Relaxed);
    let usado = REGIAO_USADO.load(Ordering::Relaxed);
    let _ = writeln!(
        t,
        "região:  {usado} de {cap} bytes recortados ({:.0}%), página {} bytes",
        porcento(usado, cap),
        REGIAO_PAGINA.load(Ordering::Relaxed)
    );
    for i in 0..painel_quantidade() {
        let Some(l) = painel_ler(i) else {
            continue;
        };
        let _ = write!(
            t,
            "arena {:<12} usado {:>10} pico {:>10} de {:>10} bytes ({:>3.0}%), {} aloc., {} falhas",
            l.nome,
            l.usado,
            l.pico,
            l.capacidade,
            porcento(l.pico, l.capacidade),
            l.alocacoes,
            l.falhas
        );
        if l.falhas > 0 {
            let _ = write!(t, " (último pedido que falhou: {} bytes)", l.ultima_falha);
        }
        let _ = t.write_char('\n');
    }
    let g = global_estatistica();
    let _ = writeln!(
        t,
        "global:  {} de {} bytes, {} alocações, {} liberações, travado: {}",
        g.usado, g.capacidade, g.alocacoes, g.liberacoes, g.travado
    );
}
