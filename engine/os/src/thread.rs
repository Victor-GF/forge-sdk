//! Threads: criacao, espera e controle da thread atual.

use core::ffi::c_void;

/// Funcao de entrada de uma thread. Recebe o `param` passado a
/// `thread_create`; o retorno vira o codigo de saida lido por `thread_join`.
pub type ThreadEntry = fn(param: *mut c_void) -> u32;

/// Thread viva criada por `thread_create`. Precisa terminar em `thread_join`
/// ou `thread_detach`; sem isso o recurso do SO vaza.
pub struct ThreadHandle(pub(crate) *mut c_void);

unsafe impl Send for ThreadHandle {}

/// Prioridade relativa as outras threads do processo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadPriority {
    /// Trabalho de fundo: carregamento, compressao.
    Low,
    #[default]
    Normal,
    /// O que nao pode atrasar: audio, a thread do frame.
    High,
}

/// Como criar uma thread. `ThreadOptions::default()` e uma thread comum, sem
/// nome, com a stack e o agendamento padrao do SO.
///
/// Num PC o agendador do SO costuma acertar sozinho, e `core` e `priority`
/// sao ajustes. Num console, os nucleos sao fixos e parte deles e do
/// sistema: fixar cada thread num nucleo, com prioridade, e o normal.
#[derive(Clone, Copy, Debug, Default)]
pub struct ThreadOptions<'a> {
    /// Visto no debugger e em profilers. Vazio: sem nome. Longo: truncado.
    pub name: &'a str,
    /// Bytes de stack reservados. Zero: o padrao do SO (1 MB no Windows).
    pub stack_size: usize,
    /// Processador logico onde a thread roda, de 0 a
    /// `logical_processor_count() - 1`. `None`: qualquer um.
    pub core: Option<u32>,
    pub priority: ThreadPriority,
}

pub trait Thread {
    /// Cria uma thread que roda `entry(param)`, ja com as `options`
    /// aplicadas antes do primeiro passo. Um ajuste de `options` que a
    /// plataforma recusar (nucleo inexistente, por exemplo) e ignorado; a
    /// thread roda mesmo assim. `None` se a thread nao pode ser criada.
    fn thread_create(entry: ThreadEntry, param: *mut c_void, options: &ThreadOptions) -> Option<ThreadHandle>;

    /// Bloqueia ate a thread terminar e devolve o codigo de saida dela.
    fn thread_join(thread: ThreadHandle) -> u32;

    /// Larga o handle sem esperar; a thread continua ate retornar sozinha.
    fn thread_detach(thread: ThreadHandle);

    /// Id da thread atual, unico entre as threads vivas do sistema.
    fn thread_current_id() -> u32;

    /// Da um nome a thread atual, como `ThreadOptions::name`: para a thread
    /// principal, que nao nasce de `thread_create`. Nomes longos sao
    /// truncados (o limite varia por plataforma; 15 bytes no Linux).
    fn thread_set_name(name: &str);

    /// Suspende a thread atual por pelo menos `ms` milissegundos.
    fn thread_sleep(ms: u32);

    /// Cede o resto da fatia de tempo da thread atual a outra pronta.
    fn thread_yield();

    /// Processadores logicos da maquina: o tamanho natural de um pool de threads.
    fn logical_processor_count() -> u32;
}
