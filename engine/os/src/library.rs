//! Bibliotecas dinamicas: carregar codigo em runtime.
//!
//! Dois usos:
//! - Hot reload: o codigo do jogo vira uma biblioteca, a engine e o
//!   executavel. Recompilar e recarregar troca o codigo sem fechar a janela
//!   nem perder o estado, que fica na memoria da engine.
//! - APIs do sistema sem linkar na compilacao: Vulkan, XInput, audio.
//!   Faltando a biblioteca, `library_open` volta `None` e o jogo decide o
//!   que fazer, em vez de nem abrir.
//!
//! Simbolos voltam como ponteiro cru: quem chama sabe a assinatura e faz o
//! `transmute` para o tipo de funcao certo, com a ABI certa (`extern "C"`
//! para codigo nosso, `extern "system"` para APIs do Windows).

use core::ffi::{CStr, c_void};

/// Biblioteca carregada por `library_open`. Precisa terminar em
/// `library_close`.
pub struct LibraryHandle(pub(crate) *mut c_void);

unsafe impl Send for LibraryHandle {}
unsafe impl Sync for LibraryHandle {}

pub trait Library {
    /// Carrega a biblioteca. Com caminho (`/` ou `\`), carrega aquele
    /// arquivo; so com o nome (`xinput1_4.dll`), busca nos diretorios do
    /// sistema. `None` se falhar.
    fn library_open(path: &str) -> Option<LibraryHandle>;

    /// Endereco do simbolo exportado `name`. Nulo se nao existir.
    fn library_symbol(library: &LibraryHandle, name: &CStr) -> *mut c_void;

    /// Descarrega a biblioteca.
    ///
    /// # Safety
    /// Nada pode estar usando a biblioteca: nenhuma thread executando codigo
    /// dela, e nenhum ponteiro para funcoes ou dados estaticos dela pode ser
    /// usado depois.
    unsafe fn library_close(library: LibraryHandle);
}
