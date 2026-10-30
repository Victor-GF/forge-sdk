//! Estatísticas de memória: o estado de todas as arenas, legível de qualquer thread.
//!
//! Cada arena tem um slot com um único escritor (a thread dona); os campos são atômicos
//! para que outra thread (o inspetor, no futuro) leia sem corrida.
//! No x86_64, load/store `Relaxed` são `mov` comuns: o custo por alocação é o mesmo de antes.
//!
//! Slots nunca são liberados (arenas vivem até o fim do programa).

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

pub const ARENAS_MAX: usize = 64;

pub(crate) struct ArenaSlot {
    active: AtomicBool,
    name: UnsafeCell<&'static str>,
    pub(crate) capacity: AtomicUsize,
    pub(crate) used: AtomicUsize,
    pub(crate) peak: AtomicUsize,
    pub(crate) allocations: AtomicU32,
    pub(crate) failures: AtomicU32,
    /// Tamanho do último pedido que não coube.
    pub(crate) last_failed_size: AtomicUsize,
    pub(crate) warned: AtomicBool,
}

// SAFETY: `name` é escrito uma vez, com o lock do registro, antes de `active` virar `true`
// (Release); leitores só o leem depois de ver `active` (Acquire).
unsafe impl Sync for ArenaSlot {}

const fn empty_slot() -> ArenaSlot {
    ArenaSlot {
        active: AtomicBool::new(false),
        name: UnsafeCell::new(""),
        capacity: AtomicUsize::new(0),
        used: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        allocations: AtomicU32::new(0),
        failures: AtomicU32::new(0),
        last_failed_size: AtomicUsize::new(0),
        warned: AtomicBool::new(false),
    }
}

static SLOTS: [ArenaSlot; ARENAS_MAX] = [const { empty_slot() }; ARENAS_MAX];
static SLOT_COUNT: AtomicUsize = AtomicUsize::new(0);
static SLOTS_LOCK: AtomicBool = AtomicBool::new(false);

// Região do programa (`MemoryRegion::reserve`), para o relatório.
pub(crate) static REGION_CAPACITY: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REGION_USED: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REGION_PAGE_SIZE: AtomicUsize = AtomicUsize::new(0);

/// Fotografia de uma arena.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArenaSnapshot {
    pub name: &'static str,
    pub capacity: usize,
    pub used: usize,
    pub peak: usize,
    pub allocations: u32,
    pub failures: u32,
    pub last_failed_size: usize,
}

/// `None` quando os `ARENAS_MAX` slots já foram usados.
pub(crate) fn register_arena(name: &'static str, capacity: usize) -> Option<&'static ArenaSlot> {
    while SLOTS_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let i = SLOT_COUNT.load(Ordering::Relaxed);
    let slot = SLOTS.get(i);
    if let Some(s) = slot {
        // SAFETY: lock pego e o slot ainda não está ativo (ninguém lê `name`).
        unsafe { *s.name.get() = name };
        s.capacity.store(capacity, Ordering::Relaxed);
        s.active.store(true, Ordering::Release);
        SLOT_COUNT.store(i.saturating_add(1), Ordering::Release);
    }
    SLOTS_LOCK.store(false, Ordering::Release);
    slot
}

pub fn arena_count() -> usize {
    SLOT_COUNT.load(Ordering::Acquire)
}

pub fn arena_snapshot(i: usize) -> Option<ArenaSnapshot> {
    let s = SLOTS.get(i)?;
    if !s.active.load(Ordering::Acquire) {
        return None;
    }
    Some(ArenaSnapshot {
        // SAFETY: `active` visto como `true`; `name` não muda mais.
        name: unsafe { *s.name.get() },
        capacity: s.capacity.load(Ordering::Relaxed),
        used: s.used.load(Ordering::Relaxed),
        peak: s.peak.load(Ordering::Relaxed),
        allocations: s.allocations.load(Ordering::Relaxed),
        failures: s.failures.load(Ordering::Relaxed),
        last_failed_size: s.last_failed_size.load(Ordering::Relaxed),
    })
}

