//! Modo automatico (`--auto`): um roteiro fixo que fecha sozinho, para testar
//! sem ninguem na frente da janela (por exemplo, rodado por um agente pelo
//! terminal). O que precisa de pessoa (teclado e mouse de verdade, ouvir o
//! som) fica de fora; o resto e exercitado e conferido.
//!
//! Roteiro, por frame:
//! - `FULLSCREEN_ON`: tela cheia. Um frame antes de `FULLSCREEN_OFF`, confere
//!   que o tamanho mudou; `FULLSCREEN_OFF` volta, e um frame antes de
//!   `KEY_START`, confere que o tamanho voltou ao original.
//! - `KEY_START..KEY_END`: `W` pressionado no input entregue ao jogo (injetado
//!   depois de `window_input`, nao no SO). O jogo loga a tecla e move o
//!   jogador.
//! - `RELOAD`: reescreve a DLL do jogo com o mesmo conteudo. O horario de
//!   escrita muda, e o hot reload precisa recarregar ate o fim.
//! - `FRAMES`: fecha. Confere o que ficou pendente e loga `[auto] ok`.
//!
//! Falha e `assert!`: o processo aborta com a mensagem no log e codigo de
//! saida diferente de zero.

use crate::hot_reload::GameCode;
use forge_os::desktop::*;
use forge_os::prelude::*;
use std::ops::Range;

const FULLSCREEN_ON: u64 = 60;
const FULLSCREEN_OFF: u64 = 90;
const KEY: Range<u64> = 120..150;
const RELOAD: u64 = 180;
const FRAMES: u64 = 300;

pub struct Auto {
    windowed: (u32, u32),
    reloads_before: u32,
}

impl Auto {
    /// `Some` se o programa foi aberto com `--auto`.
    pub fn from_args() -> Option<Auto> {
        if !std::env::args().any(|arg| arg == "--auto") {
            return None;
        }
        forge_log!("[auto] roteiro de {FRAMES} frames");
        Some(Auto { windowed: (0, 0), reloads_before: 0 })
    }

    /// Um passo do roteiro, depois de `window_input` e antes do jogo.
    /// `true` quando acabou e o loop deve fechar.
    pub fn step(&mut self, frame: u64, window: &WindowHandle, input: &mut Input, game: &GameCode) -> bool {
        let size = Platform::window_size(window);
        match frame {
            FULLSCREEN_ON => {
                self.windowed = size;
                Platform::window_set_fullscreen(window, true);
                forge_log!("[auto] tela cheia a partir de {}x{}", size.0, size.1);
            }
            _ if frame == FULLSCREEN_OFF - 1 => {
                assert!(Platform::window_is_fullscreen(window));
                assert_ne!(size, self.windowed, "tela cheia nao mudou o tamanho");
                forge_log!("[auto] tela cheia: {}x{}", size.0, size.1);
            }
            FULLSCREEN_OFF => Platform::window_set_fullscreen(window, false),
            _ if frame == KEY.start - 1 => {
                assert!(!Platform::window_is_fullscreen(window));
                assert_eq!(size, self.windowed, "sair da tela cheia nao restaurou o tamanho");
                forge_log!("[auto] janela restaurada: {}x{}", size.0, size.1);
            }
            RELOAD => {
                self.reloads_before = game.reloads();
                rewrite(game.source());
                forge_log!("[auto] DLL reescrita, esperando o hot reload");
            }
            _ => {}
        }

        // `window_input` acabou de sobrescrever o teclado: o estado injetado
        // e refeito a cada frame, com a transicao so na borda.
        let down = KEY.contains(&frame);
        let was_down = frame > 0 && KEY.contains(&(frame - 1));
        input.keyboard.keys[Key::W as usize] = Button { down, transitions: (down != was_down) as u8 };

        frame + 1 >= FRAMES
    }

    /// Confere o que so da para saber no fim.
    pub fn finish(&self, game: &GameCode) {
        assert!(
            game.reloads() > self.reloads_before,
            "o hot reload nao recarregou a DLL reescrita"
        );
        forge_log!("[auto] hot reload: {} cargas no total", game.reloads());
        forge_log!("[auto] ok");
    }
}

/// Le o arquivo inteiro e escreve de volta: mesmo conteudo, horario novo.
fn rewrite(path: &str) {
    let file = Platform::file_open(path, FileAccess::Read).expect("abrir a DLL para ler falhou");
    let size = Platform::file_size(&file).expect("file_size falhou") as usize;
    let mut bytes = vec![0u8; size];
    let read = Platform::file_read_at(&file, 0, &mut bytes);
    Platform::file_close(file);
    assert_eq!(read, Some(size), "leitura da DLL incompleta");

    let file = Platform::file_open(path, FileAccess::Write).expect("abrir a DLL para escrever falhou");
    assert!(Platform::file_write_at(&file, 0, &bytes), "reescrever a DLL falhou");
    Platform::file_close(file);
}
