//! Audio: saida de som no dispositivo padrao do sistema.
//!
//! # Formato
//!
//! Fixo: [`AUDIO_SAMPLE_RATE`] Hz, [`AUDIO_CHANNELS`] canais (estereo),
//! amostras `f32` de -1 a 1, intercaladas (`[esq, dir, esq, dir, ...]`). O
//! SO converte para o que o dispositivo usar. Um "frame de audio" e uma
//! amostra de cada canal: 2 `f32`.
//!
//! # Modelo
//!
//! Sem thread e sem callback: o jogo empurra amostras. A cada frame:
//!
//! ```ignore
//! let queued = Platform::audio_queued(&audio)?;      // ainda nao tocados
//! let target = frames_de_audio_por_frame_do_jogo * 3; // ~50 ms a frente
//! let count = target.saturating_sub(queued).min(capacity - queued);
//! mixar(&mut samples[..count as usize * AUDIO_CHANNELS]);
//! Platform::audio_write(&audio, &samples[..count as usize * AUDIO_CHANNELS]);
//! ```
//!
//! Como so o jogo escreve amostras, o codigo que as gera pode estar numa
//! biblioteca recarregada (hot reload): nada no SO guarda ponteiro para ele.
//!
//! O que ja foi escrito nao muda mais: escrever muito a frente atrasa a
//! resposta (um som disparado agora so toca depois de tudo que esta na
//! fila). Escrever pouco arrisca a fila esvaziar num frame lento (janela
//! arrastada, breakpoint): o som falha, em silencio, e nada trava.

use core::ffi::c_void;

pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: usize = 2;

/// Saida de audio aberta por `audio_open`. Usada so na thread que abriu.
/// Precisa terminar em `audio_close`.
pub struct AudioHandle(pub(crate) *mut c_void);

pub trait Audio {
    /// Abre o dispositivo de saida padrao, ja tocando (silencio ate a
    /// primeira escrita), com uma fila de pelo menos `latency_ms`. `None` se
    /// falhar (sem dispositivo de audio, por exemplo).
    fn audio_open(latency_ms: u32) -> Option<AudioHandle>;

    /// Tamanho da fila, em frames de audio. Pode ser maior que o pedido em
    /// `audio_open`: o SO arredonda.
    fn audio_capacity(audio: &AudioHandle) -> u32;

    /// Frames de audio escritos e ainda nao tocados. `None` se o dispositivo
    /// foi perdido (fone desconectado, troca do dispositivo padrao): feche e
    /// abra de novo para pegar o novo padrao.
    fn audio_queued(audio: &AudioHandle) -> Option<u32>;

    /// Poe `samples` no fim da fila. O total de frames
    /// (`samples.len() / AUDIO_CHANNELS`) precisa caber: no maximo
    /// `audio_capacity - audio_queued`. `false` se nao coube ou se o
    /// dispositivo foi perdido; nesse caso nada foi escrito.
    fn audio_write(audio: &AudioHandle, samples: &[f32]) -> bool;

    /// Para o som e libera o dispositivo.
    fn audio_close(audio: AudioHandle);
}
