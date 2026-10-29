//! Backend de janela do Linux: ainda não existe. `window_create` devolve `WindowUnsupported`
//! e o jogo usa `Platform::headless`.
//!
//! `Window` é um enum sem variantes: não dá para construir um, então as outras funções nunca
//! são chamadas (o compilador prova com `match w {}`). Quando houver backend (X11/Wayland via
//! WSLg), ele substitui este arquivo.

use crate::input::Input;
use crate::{PlatformError, WindowDesc, WindowState};

pub(crate) enum Window {}

pub(crate) fn window_create(_desc: &WindowDesc) -> Result<Window, PlatformError> {
    Err(PlatformError::WindowUnsupported)
}

pub(crate) fn window_destroy(w: Window) {
    match w {}
}

pub(crate) fn window_set_title(w: &Window, _title: &str) {
    match *w {}
}

pub(crate) fn window_poll(w: &Window, _input: &mut Input) -> WindowState {
    match *w {}
}
