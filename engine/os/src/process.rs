//! Processo: ciclo de vida do processo atual.

pub trait Process {
    /// Encerra o processo imediatamente com `code`. Nao roda destrutores.
    fn exit_process(code: u32) -> !;
}
