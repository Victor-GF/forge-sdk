//! Tempo: relogio monotonico de alta resolucao, para medir intervalos, e
//! timers para esperar ate um instante desse relogio.
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
//!
//! # Esperar
//!
//! `thread_sleep` arredonda para o tick do agendador do SO (15,6 ms no
//! Windows, por padrao): pedir 16 ms da entre 16 e 31. Para ritmo de frame,
//! use um [`TimerHandle`], que espera ate um instante absoluto de
//! `time_ticks` com precisao de cerca de 1 ms:
//!
//! ```ignore
//! let timer = Platform::timer_create().expect("timer_create falhou");
//! let frame = Platform::time_frequency() / 60;
//! let mut next = Platform::time_ticks() + frame;
//! loop {
//!     atualizar_e_desenhar();
//!     Platform::timer_wait_until(&timer, next);
//!     next += frame;
//! }
//! ```
//!
//! O prazo absoluto nao acumula erro: um frame que acordou atrasado nao
//! empurra os seguintes.

use core::ffi::c_void;

/// Timer de espera criado por `timer_create`. Uma thread por vez: pode ir
/// para outra thread, mas nao ser usado por duas ao mesmo tempo. Liberado em
/// `timer_destroy`.
pub struct TimerHandle(pub(crate) *mut c_void);

unsafe impl Send for TimerHandle {}

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

    /// Cria um timer de alta resolucao (~1 ms). Onde o SO nao tiver (Windows
    /// antes do 10 1803), cai para a resolucao do agendador. `None` se
    /// falhar.
    fn timer_create() -> Option<TimerHandle>;

    /// Dorme ate `time_ticks()` chegar em `deadline`; ja passou, volta na
    /// hora. Acorda ate ~1 ms depois do prazo. Quem precisar de mais
    /// precisao espera ate um pouco antes e gasta o resto lendo `time_ticks`.
    fn timer_wait_until(timer: &TimerHandle, deadline: u64);

    fn timer_destroy(timer: TimerHandle);
}
