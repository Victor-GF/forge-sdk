//! Dialogos: caixas modais do sistema, sem janela da engine.

use core::ffi::CStr;

pub trait Dialog {
    /// Mostra uma caixa de mensagem modal e bloqueia ate o usuario fechar.
    fn message_box(caption: &CStr, text: &CStr);
}
