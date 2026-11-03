//! Memoria: direto do SO, sem alocador intermediario.
//!
//! Duas formas:
//! - Paginas: memoria virtual em blocos de pagina. Para blocos grandes e de
//!   vida longa: a memoria do jogo, arenas, buffers de frame. Tamanhos sao
//!   arredondados para cima ate a pagina pelo SO.
//! - Heap: heap do processo, gerenciado pelo SO. Para blocos pequenos e de
//!   vida variada. Alinhamento garantido: 16 bytes.
//!
//! Falhas de alocacao voltam como ponteiro nulo ou `false`; quem chama decide
//! o que fazer.

/// Tamanhos de pagina do SO, em bytes.
#[derive(Clone, Copy)]
pub struct PageInfo {
    /// Unidade de commit/decommit e de protecao.
    pub size: usize,
    /// Alinhamento dos enderecos devolvidos por `pages_reserve`/`pages_alloc`.
    pub granularity: usize,
}

pub trait Memory {
    // ---- Paginas ----------------------------------------------------------

    fn page_info() -> PageInfo;

    /// Reserva `size` bytes de espaco de enderecos, sem memoria fisica.
    /// Acessar antes de `pages_commit` falha. Nulo se falhar.
    fn pages_reserve(size: usize) -> *mut u8;

    /// Torna utilizavel (leitura/escrita, zerada) uma faixa ja reservada.
    /// `false` se falhar; a faixa continua so reservada.
    ///
    /// # Safety
    /// `ptr..ptr+size` precisa estar dentro de uma reserva de `pages_reserve`.
    unsafe fn pages_commit(ptr: *mut u8, size: usize) -> bool;

    /// Reserva e torna utilizavel de uma vez (leitura/escrita, zerada).
    /// Nulo se falhar.
    fn pages_alloc(size: usize) -> *mut u8;

    /// Devolve a memoria fisica de uma faixa, mantendo a reserva.
    ///
    /// # Safety
    /// `ptr..ptr+size` precisa estar dentro de uma reserva viva, e nada pode
    /// acessar a faixa ate um novo `pages_commit`.
    unsafe fn pages_decommit(ptr: *mut u8, size: usize);

    /// Libera a reserva inteira.
    ///
    /// # Safety
    /// `ptr` precisa ser exatamente o devolvido por `pages_reserve` ou
    /// `pages_alloc`, e nao pode ser usado depois.
    unsafe fn pages_release(ptr: *mut u8);

    // ---- Heap -------------------------------------------------------------

    /// Aloca `size` bytes sem inicializar. Nulo se falhar.
    fn heap_alloc(size: usize) -> *mut u8;

    /// Aloca `size` bytes zerados. Nulo se falhar.
    fn heap_alloc_zeroed(size: usize) -> *mut u8;

    /// Redimensiona um bloco, movendo se preciso. Nulo se falhar; nesse caso
    /// o bloco original continua valido e ainda precisa ser liberado.
    ///
    /// # Safety
    /// `ptr` precisa ser nao-nulo, vindo de `heap_alloc*`/`heap_realloc` e
    /// ainda nao liberado. Se a chamada der certo, `ptr` deixa de valer: use
    /// o novo.
    unsafe fn heap_realloc(ptr: *mut u8, size: usize) -> *mut u8;

    /// Libera um bloco. Nulo e ignorado.
    ///
    /// # Safety
    /// `ptr` precisa vir de `heap_alloc*`/`heap_realloc`, nao liberado, e nao
    /// pode ser usado depois.
    unsafe fn heap_free(ptr: *mut u8);
}
