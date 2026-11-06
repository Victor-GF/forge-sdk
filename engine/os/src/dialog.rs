//! Dialogos: caixas modais do sistema, sem janela da engine. So em desktop
//! (`Desktop`): console nao tem caixa de mensagem do SO.

pub trait Dialog {
    /// Mostra uma caixa de mensagem modal e bloqueia ate o usuario fechar.
    fn message_box(caption: &str, text: &str);
}
