//! Sincronizacao entre threads: primitivas que bloqueiam no SO.
//!
//! Contadores e flags sem bloqueio nao passam por aqui: usam
//! `core::sync::atomic` direto.

use core::ffi::c_void;

/// Semaforo criado por `semaphore_create`. Compartilhado entre threads por
/// referencia; liberado em `semaphore_destroy`.
pub struct SemaphoreHandle(pub(crate) *mut c_void);

unsafe impl Send for SemaphoreHandle {}
unsafe impl Sync for SemaphoreHandle {}

pub trait Synchronization {
    /// Cria um semaforo com contagem `initial`, limitada a `max`.
    /// Exige `0 < max <= i32::MAX` e `initial <= max`. `None` se falhar.
    fn semaphore_create(initial: u32, max: u32) -> Option<SemaphoreHandle>;

    /// Bloqueia ate a contagem ser maior que zero e a decrementa.
    fn semaphore_wait(semaphore: &SemaphoreHandle);

    /// Soma `count` a contagem, acordando ate `count` threads em espera.
    /// Ignorado se passar de `max`.
    fn semaphore_signal(semaphore: &SemaphoreHandle, count: u32);

    /// Libera o semaforo. Nenhuma thread pode estar esperando nele.
    fn semaphore_destroy(semaphore: SemaphoreHandle);
}
