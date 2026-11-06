//! Teste de audio: um tom de 440 Hz, ligado e desligado com a tecla T.
//!
//! Mostra o modelo push: a cada frame, completa a fila ate `LEAD_FRAMES`
//! frames do jogo a frente. Mais que isso atrasaria a resposta do som; menos,
//! a fila esvaziaria num frame lento. O volume muda em rampa, sem estalo ao
//! ligar e desligar.
//!
//! Dispositivo perdido (fone desconectado, troca do padrao): fecha e tenta
//! abrir o novo padrao, de novo a cada segundo enquanto falhar.

use forge_os::prelude::*;
use std::f32::consts::TAU;

/// Fila pedida ao SO: o teto. Quanto dela fica ocupado e `LEAD_FRAMES`.
const LATENCY_MS: u32 = 100;
/// Frames do jogo de audio mantidos na fila (~50 ms a 60 fps).
const LEAD_FRAMES: u64 = 3;
const TONE_HZ: f32 = 440.0;
const VOLUME: f32 = 0.1;
/// Quanto do caminho ate o volume alvo o ganho anda por amostra: a rampa
/// leva alguns milissegundos.
const RAMP: f32 = 1.0 / 256.0;
/// Frames do jogo entre tentativas de reabrir.
const RETRY_FRAMES: u32 = 60;

pub struct AudioDemo {
    audio: Option<AudioHandle>,
    /// Frames de audio a manter na fila.
    lead: u32,
    samples: Vec<f32>,
    /// Fase do seno, em radianos.
    phase: f32,
    gain: f32,
    on: bool,
    retry_in: u32,
}

impl AudioDemo {
    pub fn new(frame_rate: u64) -> AudioDemo {
        let audio = open();
        if audio.is_none() {
            forge_log!("[audio] sem saida de audio; tentando de novo a cada segundo");
        }
        forge_log!("[audio] T liga e desliga um tom de {TONE_HZ} Hz");
        AudioDemo {
            audio,
            lead: (AUDIO_SAMPLE_RATE as u64 * LEAD_FRAMES / frame_rate) as u32,
            samples: Vec::new(),
            phase: 0.0,
            gain: 0.0,
            on: false,
            retry_in: RETRY_FRAMES,
        }
    }

    /// Uma vez por frame.
    pub fn update(&mut self, keyboard: &Keyboard) {
        if keyboard.key(Key::T).pressed() {
            self.on = !self.on;
            forge_log!("[audio] tom {}", if self.on { "ligado" } else { "desligado" });
        }

        let Some(audio) = &self.audio else {
            if self.retry_in == 0 {
                self.audio = open();
                self.retry_in = RETRY_FRAMES;
            } else {
                self.retry_in -= 1;
            }
            return;
        };
        let Some(queued) = Platform::audio_queued(audio) else {
            forge_log!("[audio] dispositivo perdido; abrindo o novo padrao");
            if let Some(audio) = self.audio.take() {
                Platform::audio_close(audio);
            }
            self.retry_in = 0;
            return;
        };

        let capacity = Platform::audio_capacity(audio);
        let count = self.lead.min(capacity).saturating_sub(queued) as usize;
        if count == 0 {
            return;
        }
        self.samples.resize(count * AUDIO_CHANNELS, 0.0);

        let target = if self.on { VOLUME } else { 0.0 };
        let step = TAU * TONE_HZ / AUDIO_SAMPLE_RATE as f32;
        for frame in self.samples.chunks_exact_mut(AUDIO_CHANNELS) {
            self.gain += (target - self.gain) * RAMP;
            frame.fill(self.phase.sin() * self.gain);
            self.phase = (self.phase + step) % TAU;
        }
        Platform::audio_write(audio, &self.samples);
    }
}

impl Drop for AudioDemo {
    fn drop(&mut self) {
        if let Some(audio) = self.audio.take() {
            Platform::audio_close(audio);
        }
    }
}

fn open() -> Option<AudioHandle> {
    let audio = Platform::audio_open(LATENCY_MS)?;
    let capacity = Platform::audio_capacity(&audio);
    forge_log!(
        "[audio] aberto: fila de {capacity} frames ({:.1} ms)",
        capacity as f32 * 1000.0 / AUDIO_SAMPLE_RATE as f32
    );
    Some(audio)
}
