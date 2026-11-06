//! Janela: a superficie onde o jogo desenha, de onde vem o input e por onde
//! chegam os eventos do sistema.
//!
//! Duas traits:
//! - [`Window`], no nucleo (`Os`): o que toda plataforma tem. No PC e uma
//!   janela do SO; num console, a tela inteira, com tamanho fixo.
//! - [`DesktopWindow`], so em desktop (`Desktop`): escala de DPI e tela
//!   cheia, que num console nao existem.
//!
//! Uma janela pertence a thread que a criou: so essa thread pode bombear os
//! eventos dela e destrui-la. Por isso `WindowHandle` nao e `Send`.
//!
//! # Pixels e escala
//!
//! Tamanhos sao em pixels de verdade, os da tela: e a resolucao em que o jogo
//! renderiza. Em monitor com escala (150% no Windows, telas "retina"), a
//! mesma janela tem mais pixels; [`DesktopWindow::window_dpi_scale`] diz
//! quanto, para a interface do jogo crescer junto. Ao mudar a janela para um
//! monitor com outra escala, o SO a redimensiona para manter o tamanho
//! visual, e `window_size` muda.

use crate::input::{Keyboard, Mouse};
use core::ffi::c_void;

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

/// Situacao do app perante o sistema, lida uma vez por frame depois de
/// `window_pump_events`. O que muda de um frame para o outro e o evento: o
/// jogo compara com o frame anterior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppState {
    /// O usuario ou o sistema pediu para encerrar: botao fechar, Alt+F4,
    /// logoff ou desligamento. Nada fecha sozinho: o jogo salva o que
    /// precisar e encerra quando quiser. Uma vez pedido, fica pedido.
    pub quit_requested: bool,
    /// O jogo esta em primeiro plano e recebe o input. Sem foco (Alt+Tab,
    /// menu do sistema num console), e o momento de pausar.
    pub focused: bool,
    /// Ha onde desenhar. Falso com a janela minimizada: pule a
    /// renderizacao.
    pub visible: bool,
    /// O sistema vai suspender o processo (console em repouso, notebook
    /// fechando a tampa): salve agora, pode nao haver outro frame. Volta a
    /// falso quando o sistema retoma.
    pub suspending: bool,
}

pub trait Window {
    /// Cria e mostra a janela, com area de desenho `width` x `height` pixels
    /// e `title` na barra de titulo. Os dois sao pedidos: onde nao ha titulo
    /// ou o tamanho e fixo (console), sao ignorados, e `window_size` diz o
    /// tamanho real. `None` se falhar.
    fn window_create(title: &str, width: u32, height: u32) -> Option<WindowHandle>;

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

    /// Situacao do app depois do ultimo `window_pump_events`.
    fn app_state(window: &WindowHandle) -> AppState;

    /// Tamanho atual da area de desenho em pixels, sem bordas nem titulo.
    /// Mudou: recrie a swapchain. `(0, 0)` com a janela minimizada: nao ha
    /// onde desenhar, pule a renderizacao.
    fn window_size(window: &WindowHandle) -> (u32, u32);

    /// Handles do SO para a API grafica. Validos ate `window_destroy`.
    fn window_native(window: &WindowHandle) -> NativeWindow;

    /// Mostra uma imagem de `width` x `height` pixels na janela: o caminho
    /// para desenhar sem GPU (framebuffer por software).
    ///
    /// `pixels` tem as linhas de cima para baixo, uma colada na outra
    /// (`width` pixels por linha), com cada pixel em `0x00RRGGBB`: na
    /// memoria, os bytes azul, verde, vermelho e um nao usado.
    ///
    /// A imagem e esticada para o maior tamanho que cabe na area de desenho
    /// sem distorcer, centralizada, com barras pretas no resto. Esticar e
    /// por vizinho mais proximo: uma imagem pequena (480x270, por exemplo)
    /// fica com pixels grandes e nitidos.
    ///
    /// A copia e feita pela CPU, a cada chamada. Serve para resolucoes
    /// modestas, ferramentas e telas de erro; a GPU e o caminho para o resto.
    ///
    /// # Panics
    /// Se `width` ou `height` for zero, ou se `pixels` tiver menos que
    /// `width * height` itens.
    fn window_present(window: &WindowHandle, pixels: &[u32], width: u32, height: u32);

    /// Entrega o input de teclado e mouse acumulado nesta janela desde a
    /// chamada anterior, e comeca a acumular o proximo frame. Uma vez por
    /// frame, depois de `window_pump_events`. Sem teclado ou mouse (console),
    /// volta tudo solto.
    ///
    /// Perder o foco solta todas as teclas e botoes: nada fica preso
    /// apertado depois de um Alt+Tab. O movimento cru (`delta_x`/`delta_y`)
    /// so chega com a janela em foco.
    fn window_input(window: &WindowHandle, keyboard: &mut Keyboard, mouse: &mut Mouse);
}

/// Janela de desktop: o que so existe num SO de janelas.
pub trait DesktopWindow {
    /// Escala do monitor onde a janela esta: 1.0 e 100% (96 dpi no Windows),
    /// 1.5 e 150%. Para dimensionar texto e interface, nao a renderizacao.
    fn window_dpi_scale(window: &WindowHandle) -> f32;

    /// Tela cheia sem borda (janela do tamanho do monitor, sem trocar o modo
    /// de video): Alt+Tab instantaneo e sem piscar. Voltar restaura a posicao
    /// e o tamanho de antes.
    fn window_set_fullscreen(window: &WindowHandle, fullscreen: bool);

    fn window_is_fullscreen(window: &WindowHandle) -> bool;
}
