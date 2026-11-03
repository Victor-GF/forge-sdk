//! Janelas: janela nativa do sistema e seus eventos.
//!
//! Uma janela pertence a thread que a criou: so essa thread pode bombear os
//! eventos dela e destrui-la. Por isso `WindowHandle` nao e `Send`.

use core::ffi::{CStr, c_void};

/// Janela criada por `window_create`. Liberada em `window_destroy`.
pub struct WindowHandle(pub(crate) *mut c_void);

pub trait Window {
    /// Cria e mostra uma janela com area de desenho `width` x `height`.
    /// `None` se falhar.
    fn window_create(title: &CStr, width: u32, height: u32) -> Option<WindowHandle>;

    /// Fecha a janela e libera seus recursos.
    fn window_destroy(window: WindowHandle);

    /// Processa todos os eventos pendentes das janelas da thread atual. Nao
    /// bloqueia: sem eventos, retorna na hora. Chamar uma vez por frame.
    fn window_pump_events();

    /// O usuario pediu para fechar (botao X, Alt+F4). A janela continua
    /// aberta ate `window_destroy`: quem decide e o jogo.
    fn window_close_requested(window: &WindowHandle) -> bool;

    /// Tamanho atual da area de desenho, sem bordas nem titulo.
    fn window_size(window: &WindowHandle) -> (u32, u32);
}
