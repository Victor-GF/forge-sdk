//! Tempo: relogio monotonico de alta resolucao, para medir intervalos.
//!
//! O contador anda em ticks de frequencia fixa, definida no boot. Nao e
//! relogio de parede: nao tem data, nao volta para tras e nao pula quando o
//! usuario ajusta a hora. O valor absoluto nao significa nada; so a diferenca
//! entre duas leituras.
//!
//! ```ignore
//! let start = Platform::time_ticks();
//! trabalho();
//! let us = Platform::time_ticks_to_seconds(Platform::time_ticks() - start) * 1e6;
//! ```

pub trait Time {
    /// Leitura atual do contador, em ticks. Barata o bastante para chamar
    /// varias vezes por frame. Leituras em threads diferentes sao comparaveis.
    fn time_ticks() -> u64;

    /// Ticks por segundo. Fixo desde o boot.
    fn time_frequency() -> u64;

    /// Converte uma diferenca de ticks em segundos.
    #[inline]
    fn time_ticks_to_seconds(ticks: u64) -> f64 {
        ticks as f64 / Self::time_frequency() as f64
    }

    /// Converte uma diferenca de ticks em nanossegundos, sem perder precisao
    /// nem estourar no meio da conta.
    #[inline]
    fn time_ticks_to_nanos(ticks: u64) -> u64 {
        (ticks as u128 * 1_000_000_000 / Self::time_frequency() as u128) as u64
    }
}
