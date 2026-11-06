//! Testa o mutex e a espera em endereco uma vez na inicializacao.
//!
//! `THREADS` threads somam 1 num contador comum, `ROUNDS` vezes cada, sem
//! atomico: so o mutex impede que somas se percam. A thread principal espera
//! todas terminarem dormindo em `finished`, sem semaforo.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use forge_os::prelude::*;

const THREADS: u32 = 4;
const ROUNDS: u64 = 100_000;

struct Shared {
    mutex: Mutex,
    /// So tocado com `mutex` travado.
    counter: UnsafeCell<u64>,
    /// Threads que ja terminaram as somas.
    finished: AtomicU32,
}

fn add(param: *mut c_void) -> u32 {
    let shared = unsafe { &*(param as *const Shared) };
    for _ in 0..ROUNDS {
        Platform::mutex_lock(&shared.mutex);
        unsafe {
            *shared.counter.get() += 1;
            Platform::mutex_unlock(&shared.mutex);
        }
    }
    // Grava primeiro, acorda depois: quem acordar ja ve o valor novo.
    shared.finished.fetch_add(1, Ordering::Release);
    Platform::address_wake_all(&shared.finished);
    0
}

pub fn run() {
    let shared = Shared {
        mutex: Mutex::new(),
        counter: UnsafeCell::new(0),
        finished: AtomicU32::new(0),
    };
    let param: *mut c_void = (&raw const shared).cast_mut().cast();

    let start = Platform::time_ticks();
    let threads: Vec<ThreadHandle> = (0..THREADS)
        .map(|index| {
            let name = format!("forge sync {index}");
            let options = ThreadOptions { name: &name, ..ThreadOptions::default() };
            Platform::thread_create(add, param, &options).expect("thread_create falhou")
        })
        .collect();

    let mut seen = shared.finished.load(Ordering::Acquire);
    while seen < THREADS {
        Platform::address_wait(&shared.finished, seen);
        seen = shared.finished.load(Ordering::Acquire);
    }
    let elapsed = Platform::time_ticks() - start;

    // As somas acabaram; o join espera as threads sairem de `add` antes de
    // `shared` deixar de existir.
    for thread in threads {
        Platform::thread_join(thread);
    }

    let total = unsafe { *shared.counter.get() };
    assert_eq!(total, THREADS as u64 * ROUNDS, "o mutex deixou somas se perderem");
    forge_log!(
        "[sync] ok: {THREADS} threads x {ROUNDS} somas com mutex em {:.1} ms",
        Platform::time_ticks_to_seconds(elapsed) * 1000.0
    );
}
