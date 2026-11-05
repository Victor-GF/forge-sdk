//! Janelas: janela nativa do sistema e seus eventos.
//!
//! Uma janela pertence a thread que a criou: so essa thread pode bombear os
//! eventos dela e destrui-la. Por isso `WindowHandle` nao e `Send`.
//!
//! # Pixels e escala
//!
//! Tamanhos sao em pixels de verdade, os da tela: e a resolucao em que o jogo
//! renderiza. Em monitor com escala (150% no Windows, telas "retina"), a
//! mesma janela tem mais pixels; [`Window::window_dpi_scale`] diz quanto,
//! para a interface do jogo crescer junto. Ao mudar a janela para um monitor
//! com outra escala, o SO a redimensiona para manter o tamanho visual, e
//! `window_size` muda.

use crate::input::{Keyboard, Mouse};
use core::ffi::{CStr, c_void};

/// Janela criada por `window_create`. Liberada em `window_destroy`.
pub struct WindowHandle(pub(crate) *mut c_void);

/// Handles do SO para APIs graficas criarem a superficie de desenho
/// (swapchain). Uma variante por sistema de janelas; quem usa casa so a da
/// plataforma que suporta.
#[derive(Clone, Copy, Debug)]
pub enum NativeWindow {
    /// `VkWin32SurfaceCreateInfoKHR` pede os dois; D3D12/DXGI, so o `hwnd`.
    Win32 { hinstance: *mut c_void, hwnd: *mut c_void },
}

pub trait Window {
    /// Cria e mostra uma janela com area de desenho `width` x `height`
    /// pixels. `None` se falhar.
    fn window_create(title: &CStr, width: u32, height: u32) -> Option<WindowHandle>;

    /// Fecha a janela e libera seus recursos. Antes disso, destrua o que a
    /// API grafica criou sobre ela.
    fn window_destroy(window: WindowHandle);

    /// Processa todos os eventos pendentes das janelas da thread atual. Nao
    /// bloqueia: sem eventos, retorna na hora. Chamar uma vez por frame.
    ///
    /// No Windows, enquanto o usuario arrasta ou redimensiona a janela, o
    /// sistema fica com a thread dentro desta chamada: o frame para ate ele
    /// soltar.
    fn window_pump_events();

    /// O usuario pediu para fechar (botao X, Alt+F4). A janela continua
    /// aberta ate `window_destroy`: quem decide e o jogo.
    fn window_close_requested(window: &WindowHandle) -> bool;

    /// Tamanho atual da area de desenho em pixels, sem bordas nem titulo.
    /// Mudou: recrie a swapchain. `(0, 0)` com a janela minimizada: nao ha
    /// onde desenhar, pule a renderizacao.
    fn window_size(window: &WindowHandle) -> (u32, u32);

    /// Escala do monitor onde a janela esta: 1.0 e 100% (96 dpi no Windows),
    /// 1.5 e 150%. Para dimensionar texto e interface, nao a renderizacao.
    fn window_dpi_scale(window: &WindowHandle) -> f32;

    /// Tela cheia sem borda (janela do tamanho do monitor, sem trocar o modo
    /// de video): Alt+Tab instantaneo e sem piscar. Voltar restaura a posicao
    /// e o tamanho de antes.
    fn window_set_fullscreen(window: &WindowHandle, fullscreen: bool);

    fn window_is_fullscreen(window: &WindowHandle) -> bool;

    /// Handles do SO para a API grafica. Validos ate `window_destroy`.
    fn window_native(window: &WindowHandle) -> NativeWindow;

    /// Entrega o input de teclado e mouse acumulado nesta janela desde a
    /// chamada anterior, e comeca a acumular o proximo frame. Uma vez por
    /// frame, depois de `window_pump_events`.
    ///
    /// Perder o foco solta todas as teclas e botoes: nada fica preso
    /// apertado depois de um Alt+Tab. O movimento cru (`delta_x`/`delta_y`)
    /// so chega com a janela em foco.
    fn window_input(window: &WindowHandle, keyboard: &mut Keyboard, mouse: &mut Mouse);
}
