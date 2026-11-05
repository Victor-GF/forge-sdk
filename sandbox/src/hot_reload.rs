//! Hot reload do codigo do jogo (`sandbox/game`, uma DLL).
//!
//! O Windows trava a DLL carregada, e o compilador nao conseguiria
//! sobrescreve-la. Por isso o sandbox carrega sempre uma copia, e a original
//! fica livre para o `cargo build`.
//!
//! As copias alternam entre dois nomes. A nova e carregada com a antiga ainda
//! carregada, e so se ela abrir e tiver `game_update` a antiga e
//! descarregada. Uma DLL quebrada ou pela metade nao derruba o jogo: ele segue
//! com o codigo antigo e tenta de novo na proxima mudanca.
//!
//! O compilador escreve a DLL aos poucos. Para nao copiar um arquivo pela
//! metade, so recarrega depois que o horario de escrita ficou parado por
//! `SETTLE_FRAMES` frames.

use core::ffi::c_void;
use core::mem;
use forge_os::file::{PATH_CAPACITY, path_parent};
use forge_os::prelude::*;

/// Assinatura de `game_update` em `sandbox/game/src/lib.rs`.
type GameUpdate = unsafe extern "C" fn(memory: *mut u8, size: usize, input: *const Input, frame: u64);

/// Frames sem mudanca no horario da DLL antes de recarregar (~160 ms).
const SETTLE_FRAMES: u32 = 10;

pub struct GameCode {
    /// DLL gerada pelo compilador.
    source: String,
    /// Copias carregadas, alternando.
    copies: [String; 2],
    /// Qual de `copies` esta carregada.
    current: usize,
    library: Option<LibraryHandle>,
    update: Option<GameUpdate>,
    /// Horario de escrita da ultima tentativa de carga, deu certo ou nao.
    attempted: u64,
    /// Ultimo horario visto, e ha quantos frames ele nao muda.
    seen: u64,
    stable_frames: u32,
}

impl GameCode {
    /// `name` e o arquivo da DLL, no diretorio do executavel.
    pub fn new(name: &str) -> GameCode {
        let mut exe_buf = [0u8; PATH_CAPACITY];
        let exe = Platform::path_executable(&mut exe_buf).expect("path_executable falhou");
        let dir = path_parent(exe);
        let stem = name.strip_suffix(".dll").unwrap_or(name);
        let mut code = GameCode {
            source: format!("{dir}/{name}"),
            copies: [format!("{dir}/{stem}.loaded0.dll"), format!("{dir}/{stem}.loaded1.dll")],
            // A primeira carga vai para `copies[0]`.
            current: 1,
            library: None,
            update: None,
            attempted: 0,
            seen: 0,
            stable_frames: 0,
        };
        match Platform::file_modified(&code.source) {
            Some(modified) => code.load(modified),
            None => forge_log!(
                "[hot] {} nao existe: compile com cargo build -p forge_sandbox_game",
                code.source
            ),
        }
        code
    }

    /// Chama o `game_update` carregado. Sem DLL, nao faz nada.
    ///
    /// # Safety
    /// O contrato de `game_update`: `memory` com `size` bytes, alinhada a
    /// pagina, zerada antes da primeira chamada e so do jogo.
    pub unsafe fn update(&self, memory: *mut u8, size: usize, input: &Input, frame: u64) {
        if let Some(update) = self.update {
            unsafe { update(memory, size, input, frame) }
        }
    }

    /// Recarrega se a DLL mudou e ja parou de mudar. Uma vez por frame, na
    /// mesma thread de `update` e fora dela: ninguem esta no codigo antigo
    /// quando ele e descarregado.
    pub fn poll(&mut self) {
        let Some(modified) = Platform::file_modified(&self.source) else {
            return;
        };
        if modified != self.seen {
            self.seen = modified;
            self.stable_frames = 0;
            return;
        }
        if modified == self.attempted {
            return;
        }
        self.stable_frames += 1;
        if self.stable_frames >= SETTLE_FRAMES {
            self.load(modified);
        }
    }

    fn load(&mut self, modified: u64) {
        self.attempted = modified;
        self.seen = modified;
        let next = 1 - self.current;
        let copy = &self.copies[next];

        if !Platform::file_copy(&self.source, copy) {
            forge_log!("[hot] falha ao copiar para {copy}");
            return;
        }
        let Some(library) = Platform::library_open(copy) else {
            forge_log!("[hot] falha ao carregar {copy}");
            return;
        };
        let symbol = Platform::library_symbol(&library, c"game_update");
        if symbol.is_null() {
            forge_log!("[hot] {copy} nao exporta game_update");
            unsafe { Platform::library_close(library) };
            return;
        }
        // O simbolo e o `game_update` do jogo, com a assinatura de `GameUpdate`.
        let update = unsafe { mem::transmute::<*mut c_void, GameUpdate>(symbol) };

        self.update = Some(update);
        if let Some(old) = self.library.replace(library) {
            unsafe { Platform::library_close(old) };
        }
        self.current = next;
        forge_log!("[hot] carregado {copy}");
    }
}

impl Drop for GameCode {
    fn drop(&mut self) {
        self.update = None;
        if let Some(library) = self.library.take() {
            unsafe { Platform::library_close(library) };
        }
    }
}
