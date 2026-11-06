//! Sincronizacao entre threads: primitivas que bloqueiam no SO.
//!
//! Contadores e flags sem bloqueio nao passam por aqui: usam
//! `core::sync::atomic` direto.
//!
//! Tres ferramentas, da mais pesada para a mais leve:
//! - Semaforo: objeto do kernel, com handle. Toda operacao e uma chamada ao
//!   SO, mesmo sem ninguem esperando.
//! - [`Mutex`]: exclusao mutua sem objeto do kernel. Sem disputa, travar e
//!   destravar sao atomicos em user mode; o SO so entra quando uma thread
//!   precisa dormir.
//! - Espera em endereco (`address_wait`/`address_wake_*`): dormir ate um
//!   `AtomicU32` mudar. E a base para construir primitivas proprias
//!   (eventos, barreiras, contadores de conclusao); o SO so entra quando
//!   alguem de fato dorme ou precisa ser acordado.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::AtomicU32;

/// Semaforo criado por `semaphore_create`. Compartilhado entre threads por
/// referencia; liberado em `semaphore_destroy`.
pub struct SemaphoreHandle(pub(crate) *mut c_void);

unsafe impl Send for SemaphoreHandle {}
unsafe impl Sync for SemaphoreHandle {}

/// Mutex leve: o tamanho de um ponteiro, sem criar nem destruir. Comeca
/// destravado (`Mutex::new`), e pode ser um `static`.
///
/// Nao protege dado nenhum por si: e so a trava. Quem usa garante que o dado
/// so e tocado entre `mutex_lock` e `mutex_unlock`. Nao e reentrante: travar
/// de novo na mesma thread trava para sempre.
///
/// Nao pode ser movido nem descartado enquanto travado ou com alguem
/// esperando: o SO guarda o endereco.
pub struct Mutex(pub(crate) UnsafeCell<usize>);

unsafe impl Sync for Mutex {}

impl Mutex {
    #[inline]
    pub const fn new() -> Mutex {
        Mutex(UnsafeCell::new(0))
    }
}

impl Default for Mutex {
    fn default() -> Mutex {
        Mutex::new()
    }
}

pub trait Synchronization {
    // ---- Semaforo ---------------------------------------------------------

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

    // ---- Mutex ------------------------------------------------------------

    /// Trava, esperando se outra thread estiver com o mutex.
    fn mutex_lock(mutex: &Mutex);

    /// Trava se estiver livre, sem esperar. `true` se travou.
    fn mutex_try_lock(mutex: &Mutex) -> bool;

    /// Destrava.
    ///
    /// # Safety
    /// A thread atual precisa estar com o mutex, travado por `mutex_lock` ou
    /// por um `mutex_try_lock` que devolveu `true`.
    unsafe fn mutex_unlock(mutex: &Mutex);

    // ---- Espera em endereco -----------------------------------------------

    /// Dorme enquanto `address` valer `expected`; se ja for outro valor,
    /// volta na hora. Pode voltar sem que ninguem tenha chamado `wake` (o SO
    /// permite acordar sem motivo): quem chama confere o valor num loop.
    ///
    /// ```ignore
    /// let mut seen = done.load(Ordering::Acquire);
    /// while seen < total {
    ///     Platform::address_wait(&done, seen);
    ///     seen = done.load(Ordering::Acquire);
    /// }
    /// ```
    ///
    /// Do lado de quem muda o valor: grava primeiro, acorda depois.
    fn address_wait(address: &AtomicU32, expected: u32);

    /// Acorda uma thread dormindo em `address`, se houver.
    fn address_wake_one(address: &AtomicU32);

    /// Acorda todas as threads dormindo em `address`.
    fn address_wake_all(address: &AtomicU32);
}
