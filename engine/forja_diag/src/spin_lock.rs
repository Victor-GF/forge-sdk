//! Spin lock mínimo para os poucos statics do diagnóstico.

use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct SpinLock(AtomicBool);

impl SpinLock {
    pub(crate) const fn new() -> SpinLock {
        SpinLock(AtomicBool::new(false))
    }

    /// Tenta pegar o lock girando no máximo `attempts` vezes. O caminho de crash usa poucas
    /// tentativas para nunca travar se a própria thread morreu segurando o lock.
    pub(crate) fn try_lock(&self, attempts: u32) -> bool {
        for _ in 0..attempts {
            if self.0.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    pub(crate) fn unlock(&self) {
        self.0.store(false, Ordering::Release);
    }
}
