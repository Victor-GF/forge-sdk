//! Threads: criacao, espera e controle da thread atual.

use core::ffi::c_void;

/// Funcao de entrada de uma thread. Recebe o `param` passado a
/// `thread_create`; o retorno vira o codigo de saida lido por `thread_join`.
pub type ThreadEntry = fn(param: *mut c_void) -> u32;

/// Thread viva criada por `thread_create`. Precisa terminar em `thread_join`
/// ou `thread_detach`; sem isso o recurso do SO vaza.
pub struct ThreadHandle(pub(crate) *mut c_void);

unsafe impl Send for ThreadHandle {}

pub trait Thread {
    /// Cria uma thread que roda `entry(param)` imediatamente. `None` se falhar.
    fn thread_create(entry: ThreadEntry, param: *mut c_void) -> Option<ThreadHandle>;

    /// Bloqueia ate a thread terminar e devolve o codigo de saida dela.
    fn thread_join(thread: ThreadHandle) -> u32;

    /// Larga o handle sem esperar; a thread continua ate retornar sozinha.
    fn thread_detach(thread: ThreadHandle);

    /// Id da thread atual, unico entre as threads vivas do sistema.
    fn thread_current_id() -> u32;

    /// Suspende a thread atual por pelo menos `ms` milissegundos.
    fn thread_sleep(ms: u32);

    /// Cede o resto da fatia de tempo da thread atual a outra pronta.
    fn thread_yield();

    /// Processadores logicos da maquina: o tamanho natural de um pool de threads.
    fn logical_processor_count() -> u32;
}
