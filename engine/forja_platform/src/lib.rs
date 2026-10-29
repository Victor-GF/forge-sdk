//! Plataforma: janela, input e ritmo de frame.
//!
//! **Este arquivo é o contrato.** A janela tem um backend por SO (`windows.rs`: Win32;
//! `linux.rs`: ainda sem janela, `init` devolve `WindowUnsupported`). Relógio e sleep vêm do
//! `forja_os`.
//!
//! O jogo é dono do loop: chama `Platform::poll` uma vez por frame e decide o resto. Dois modos:
//! - `Platform::init`: com janela; o input vem do SO.
//! - `Platform::headless(max_frames)`: sem janela, em qualquer SO; `poll` devolve `false`
//!   depois de `max_frames` frames. Para testes, depuração e profiling da simulação.
//!
//! Não aloca. Nenhum estado global.

pub mod input;

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
mod sys;

use forja_os::time;

use input::Input;

pub struct WindowDesc<'a> {
    pub title: &'a str,
    /// Tamanho da área cliente, em pixels físicos.
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlatformError {
    /// Título maior que `TITLE_MAX` em UTF-16.
    TitleTooLong,
    RegisterClass,
    CreateWindow,
    /// Este SO ainda não tem backend de janela (use `Platform::headless`).
    WindowUnsupported,
}

/// Tamanho máximo de título (em unidades UTF-16, com o 0 final).
pub const TITLE_MAX: usize = 256;

/// Estado da janela lido em cada `poll` (contrato entre `Platform` e o backend).
pub(crate) struct WindowState {
    pub(crate) closed: bool,
    pub(crate) focused: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Janela aberta (ou modo headless). Só existe depois de `init`/`headless`; `shutdown`
/// consome o valor.
pub struct Platform {
    /// `None` no modo headless.
    window: Option<sys::Window>,
    /// Modo headless: `poll` devolve `false` quando `frame_index` chega aqui.
    max_frames: u64,

    /// Fica `false` quando o usuário fecha a janela (ou acabam os frames do modo headless).
    pub running: bool,
    pub focused: bool,
    /// Área cliente atual; 0x0 quando minimizada ou headless.
    pub width: u32,
    pub height: u32,
    /// `true` no frame em que o tamanho mudou.
    pub resized: bool,

    pub input: Input,

    pub ticks_per_second: u64,
    /// Tick em que o frame atual começou (`poll`).
    pub frame_start: u64,
    pub frame_index: u64,
    /// Segundos desde o `poll` anterior. Sem limite: o jogo decide se limita.
    pub dt: f32,
}

impl Platform {
    fn new(window: Option<sys::Window>, width: u32, height: u32, max_frames: u64) -> Platform {
        time::begin_precise_sleep();
        Platform {
            window,
            max_frames,
            running: true,
            focused: true,
            width,
            height,
            resized: false,
            input: Input::EMPTY,
            ticks_per_second: time::ticks_per_second(),
            frame_start: time::ticks(),
            frame_index: 0,
            dt: 0.0,
        }
    }

    /// Abre a janela. `WindowUnsupported` em SO sem backend de janela.
    pub fn init(desc: &WindowDesc) -> Result<Platform, PlatformError> {
        let window = sys::window_create(desc)?;
        Ok(Platform::new(Some(window), desc.width, desc.height, u64::MAX))
    }

    /// Sem janela e sem input do SO. `poll` devolve `false` depois de `max_frames` frames.
    pub fn headless(max_frames: u64) -> Platform {
        Platform::new(None, 0, 0, max_frames)
    }

    pub fn is_headless(&self) -> bool {
        self.window.is_none()
    }

    pub fn shutdown(self) {
        if let Some(w) = self.window {
            sys::window_destroy(w);
        }
        time::end_precise_sleep();
    }

    /// Títulos maiores que `TITLE_MAX` são ignorados. No modo headless não faz nada.
    pub fn set_title(&self, title: &str) {
        if let Some(w) = &self.window {
            sys::window_set_title(w, title);
        }
    }

    /// Processa as mensagens do SO e atualiza input, tamanho, foco e tempo.
    /// Devolve `false` quando a janela foi fechada (ou acabaram os frames do modo headless).
    pub fn poll(&mut self) -> bool {
        self.input.begin_frame();

        match &self.window {
            Some(w) => {
                let s = sys::window_poll(w, &mut self.input);
                if s.closed {
                    self.running = false;
                }
                if self.focused && !s.focused {
                    self.input.release_all();
                }
                self.focused = s.focused;
                self.resized = s.width != self.width || s.height != self.height;
                self.width = s.width;
                self.height = s.height;
            }
            None => {
                if self.frame_index >= self.max_frames {
                    self.running = false;
                }
            }
        }

        let now = time::ticks();
        self.dt = time::ticks_to_seconds(now.saturating_sub(self.frame_start), self.ticks_per_second) as f32;
        self.frame_start = now;
        self.frame_index = self.frame_index.wrapping_add(1);

        self.running
    }

    /// Dorme até `target_seconds` depois do início do frame. Usa sleep enquanto falta
    /// bastante e gira (spin) nos últimos ~2 ms, porque o sleep do SO não é preciso.
    pub fn wait_frame(&self, target_seconds: f64) {
        let target_ticks = (target_seconds * self.ticks_per_second as f64) as u64;
        let end = self.frame_start.saturating_add(target_ticks);
        loop {
            let now = time::ticks();
            if now >= end {
                return;
            }
            let left_ms = time::ticks_to_seconds(end.saturating_sub(now), self.ticks_per_second) * 1000.0;
            if left_ms > 2.0 {
                time::sleep_ms((left_ms - 1.5) as u32);
            } else {
                core::hint::spin_loop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_runs_exactly_max_frames() {
        let mut p = Platform::headless(3);
        let mut frames = 0u32;
        while p.poll() {
            frames = frames.saturating_add(1);
            assert!(frames < 10);
        }
        assert_eq!(frames, 3);
        assert!(p.is_headless());
        p.set_title("ignorado");
        p.shutdown();
    }
}
