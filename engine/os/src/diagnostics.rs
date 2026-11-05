//! Diagnostico: saida de texto e controle do debugger.
//!
//! Dois destinos de texto, independentes:
//! - Console: a saida padrao do processo (terminal, ou arquivo/pipe se
//!   redirecionada). Sem console, o texto e descartado.
//! - Debugger: a janela de saida do debugger anexado (Output no Visual
//!   Studio, RemedyBG, WinDbg). Funciona mesmo em app sem console. Sem
//!   debugger, o texto e descartado.
//!
//! Para log formatado sem `std`, use [`forge_log!`](crate::forge_log!), que
//! escreve nos dois.

pub trait Diagnostics {
    /// Escreve `text` como esta, sem quebra de linha, na saida padrao.
    fn console_write(text: &str);

    /// Escreve `text` como esta, sem quebra de linha, na saida do debugger.
    fn debug_write(text: &str);

    /// Ha um debugger anexado ao processo agora.
    fn debugger_present() -> bool;

    /// Para no debugger, nesta linha. Sem debugger anexado, derruba o
    /// processo: chame so depois de `debugger_present`.
    fn debug_break();
}
