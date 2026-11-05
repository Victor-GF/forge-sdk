//! Codigo do jogo do sandbox, carregado como biblioteca dinamica (hot reload).
//!
//! O sandbox carrega esta DLL, chama `game_update` a cada frame e recarrega a
//! DLL quando ela e recompilada. Para ver funcionando: com o sandbox aberto,
//! mude `MESSAGE` e rode `cargo build -p forge_sandbox_game` (com o mesmo
//! `--target` do sandbox). A mensagem muda e o contador continua de onde
//! estava.
//!
//! Todo o estado fica em `memory`, que pertence ao executavel e sobrevive a
//! troca da DLL. Nada de `static mut` aqui: seria zerado a cada reload. Mudar
//! o layout de `State` exige reiniciar o sandbox.
//!
//! Input de teste: teclas apertadas sao logadas; WASD, setas ou o analogico
//! esquerdo movem o "jogador"; clique esquerdo loga a posicao do cursor; a
//! roda loga o giro; arrastar com o botao direito loga o movimento cru; o
//! botao `South` do gamepad (A no Xbox) loga analogicos e gatilhos.

use forge_os::forge_log;
use forge_os::input::{GamepadButton, Input, Key, MouseButton};

const MESSAGE: &str = "versao 7";

/// A cada quantos frames loga o estado.
const LOG_EVERY: u64 = 60;

/// Unidades por frame com a direcao no maximo.
const SPEED: f32 = 0.1;

#[repr(C)]
struct State {
    initialized: bool,
    updates: u64,
    /// Posicao do jogador, com y para cima.
    x: f32,
    y: f32,
    /// Movimento cru do mouse somado enquanto o botao direito esta apertado.
    drag_x: i32,
    drag_y: i32,
}

/// Chamado pelo sandbox uma vez por frame. A assinatura e um contrato com
/// `GameUpdate` em `sandbox/src/hot_reload.rs`: mudou aqui, muda la.
///
/// # Safety
/// `memory` aponta para `size` bytes alinhados a pagina, zerados antes da
/// primeira chamada e usados so por esta funcao. `input` e valido durante a
/// chamada.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn game_update(memory: *mut u8, size: usize, input: *const Input, frame: u64) {
    assert!(size >= size_of::<State>(), "memoria do jogo pequena demais");
    let state = unsafe { &mut *(memory as *mut State) };
    let input = unsafe { &*input };
    if !state.initialized {
        state.initialized = true;
        forge_log!("[game] estado inicializado");
    }
    state.updates += 1;

    let keyboard = &input.keyboard;
    for (index, button) in keyboard.keys.iter().enumerate() {
        if let (true, Some(key)) = (button.pressed(), Key::from_index(index)) {
            forge_log!("[game] tecla {key:?}");
        }
    }

    // Direcao: teclado e analogico esquerdo de qualquer gamepad, somados.
    let held = |a: Key, b: Key| keyboard.key(a).down || keyboard.key(b).down;
    let mut dx = 0.0;
    let mut dy = 0.0;
    if held(Key::A, Key::Left) {
        dx -= 1.0;
    }
    if held(Key::D, Key::Right) {
        dx += 1.0;
    }
    if held(Key::W, Key::Up) {
        dy += 1.0;
    }
    if held(Key::S, Key::Down) {
        dy -= 1.0;
    }
    for pad in input.gamepads.iter().filter(|pad| pad.connected) {
        dx += pad.left_x;
        dy += pad.left_y;
    }
    state.x += dx * SPEED;
    state.y += dy * SPEED;

    let mouse = &input.mouse;
    if mouse.button(MouseButton::Left).pressed() {
        forge_log!("[game] clique em ({}, {})", mouse.x, mouse.y);
    }
    if mouse.wheel != 0.0 {
        forge_log!("[game] roda {:+.1}", mouse.wheel);
    }
    let right = mouse.button(MouseButton::Right);
    if right.pressed() {
        state.drag_x = 0;
        state.drag_y = 0;
    }
    if right.down || right.released() {
        state.drag_x += mouse.delta_x;
        state.drag_y += mouse.delta_y;
    }
    if right.released() {
        forge_log!("[game] arrastou ({}, {}) cru", state.drag_x, state.drag_y);
    }

    for (index, pad) in input.gamepads.iter().enumerate() {
        if pad.button(GamepadButton::South).pressed() {
            forge_log!(
                "[game] gamepad {index}: esquerdo ({:.2}, {:.2}), direito ({:.2}, {:.2}), gatilhos ({:.2}, {:.2})",
                pad.left_x,
                pad.left_y,
                pad.right_x,
                pad.right_y,
                pad.left_trigger,
                pad.right_trigger,
            );
        }
    }

    if frame % LOG_EVERY == 0 {
        forge_log!(
            "[game] {MESSAGE}: {} updates, jogador em ({:.1}, {:.1})",
            state.updates,
            state.x,
            state.y
        );
    }
}
