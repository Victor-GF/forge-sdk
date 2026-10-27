//! Counting allocator: wraps the system allocator and keeps totals, so telemetry can show
//! allocations per frame, bytes live and the peak. A binary opts in with
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: forja::alloc::CountingAlloc = forja::alloc::CountingAlloc;
//! ```
//!
//! Each thread also counts its own allocations in a thread-local, which `profile` reads at scope
//! start and end to charge allocations to the scope that made them.
//!
//! Cost: a few relaxed atomic adds per allocation. Without the opt-in every counter stays 0.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

pub struct CountingAlloc;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static THREAD_ALLOCS: Cell<u64> = const { Cell::new(0) };
}

fn on_alloc(size: usize) {
    ALLOCS.fetch_add(1, Relaxed);
    BYTES_ALLOCATED.fetch_add(size as u64, Relaxed);
    let live = LIVE.fetch_add(size as u64, Relaxed) + size as u64;
    PEAK.fetch_max(live, Relaxed);
    // try_with: the thread may be tearing down its locals.
    let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get() + 1));
}

fn on_free(size: usize) {
    FREES.fetch_add(1, Relaxed);
    LIVE.fetch_sub(size as u64, Relaxed);
}

// SAFETY: forwards every call to `System` unchanged; the counters have no effect on the memory.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            on_alloc(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            on_alloc(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        on_free(layout.size());
    }

    /// Counted as a free plus an allocation: a growing `Vec` is an allocation event.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            on_free(layout.size());
            on_alloc(new_size);
        }
        p
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocStats {
    /// Allocations (and reallocations) since start.
    pub allocs: u64,
    pub frees: u64,
    /// Total bytes requested since start.
    pub bytes_allocated: u64,
    pub live_bytes: u64,
    pub peak_bytes: u64,
}

impl AllocStats {
    /// Counter differences from `earlier` to `self` (live and peak are kept as in `self`).
    pub fn since(&self, earlier: &AllocStats) -> AllocStats {
        AllocStats {
            allocs: self.allocs - earlier.allocs,
            frees: self.frees - earlier.frees,
            bytes_allocated: self.bytes_allocated - earlier.bytes_allocated,
            ..*self
        }
    }
}

pub fn stats() -> AllocStats {
    AllocStats {
        allocs: ALLOCS.load(Relaxed),
        frees: FREES.load(Relaxed),
        bytes_allocated: BYTES_ALLOCATED.load(Relaxed),
        live_bytes: LIVE.load(Relaxed),
        peak_bytes: PEAK.load(Relaxed),
    }
}

/// Allocations made by the calling thread so far.
pub fn thread_allocs() -> u64 {
    THREAD_ALLOCS.with(|c| c.get())
}

/// True once anything went through `CountingAlloc` (i.e. the binary installed it).
pub fn installed() -> bool {
    ALLOCS.load(Relaxed) > 0
}
