//! Trava de giro (spin lock) mínima para os poucos statics do diagnóstico.

use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct Trava(AtomicBool);

pub(crate) const fn trava_nova() -> Trava {
    Trava(AtomicBool::new(false))
}

/// Tenta pegar a trava girando no máximo `tentativas` vezes. O caminho de crash usa poucas
/// tentativas para nunca travar se a própria thread morreu segurando a trava.
pub(crate) fn trava_tentar(t: &Trava, tentativas: u32) -> bool {
    for _ in 0..tentativas {
        if t.0.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

pub(crate) fn trava_soltar(t: &Trava) {
    t.0.store(false, Ordering::Release);
}
