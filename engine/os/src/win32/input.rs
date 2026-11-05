//! Teclado (traducao de scancode, usada pela janela) e gamepads (XInput).

use super::{Win32, ffi};
use crate::input::{GAMEPAD_BUTTON_COUNT, GAMEPAD_COUNT, Gamepad, GamepadButton, Gamepads, Key};
use crate::library::Library;
use core::ffi::c_void;
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

// ---- Teclado ----------------------------------------------------------------

/// Tecla pelo scancode (conjunto 1, o que vem em `lParam`), que e a posicao
/// fisica e nao muda com o layout. `extended` e o prefixo `E0`: separa, por
/// exemplo, o Ctrl direito do esquerdo e as setas das do teclado numerico.
/// Teclas fora de [`Key`] (teclado numerico, pontuacao) voltam `None`.
pub(super) fn key_from_scancode(scancode: u8, extended: bool) -> Option<Key> {
    use Key::*;
    let key = if extended {
        match scancode {
            0x1C => Enter,
            0x1D => RightControl,
            0x38 => RightAlt,
            0x47 => Home,
            0x48 => Up,
            0x49 => PageUp,
            0x4B => Left,
            0x4D => Right,
            0x4F => End,
            0x50 => Down,
            0x51 => PageDown,
            0x52 => Insert,
            0x53 => Delete,
            _ => return None,
        }
    } else {
        match scancode {
            0x01 => Escape,
            0x02 => Digit1,
            0x03 => Digit2,
            0x04 => Digit3,
            0x05 => Digit4,
            0x06 => Digit5,
            0x07 => Digit6,
            0x08 => Digit7,
            0x09 => Digit8,
            0x0A => Digit9,
            0x0B => Digit0,
            0x0C => Minus,
            0x0D => Equals,
            0x0E => Backspace,
            0x0F => Tab,
            0x10 => Q,
            0x11 => W,
            0x12 => E,
            0x13 => R,
            0x14 => T,
            0x15 => Y,
            0x16 => U,
            0x17 => I,
            0x18 => O,
            0x19 => P,
            0x1C => Enter,
            0x1D => LeftControl,
            0x1E => A,
            0x1F => S,
            0x20 => D,
            0x21 => F,
            0x22 => G,
            0x23 => H,
            0x24 => J,
            0x25 => K,
            0x26 => L,
            0x29 => Grave,
            0x2A => LeftShift,
            0x2C => Z,
            0x2D => X,
            0x2E => C,
            0x2F => V,
            0x30 => B,
            0x31 => N,
            0x32 => M,
            0x36 => RightShift,
            0x38 => LeftAlt,
            0x39 => Space,
            0x3B => F1,
            0x3C => F2,
            0x3D => F3,
            0x3E => F4,
            0x3F => F5,
            0x40 => F6,
            0x41 => F7,
            0x42 => F8,
            0x43 => F9,
            0x44 => F10,
            0x57 => F11,
            0x58 => F12,
            _ => return None,
        }
    };
    Some(key)
}

// ---- Gamepads (XInput) ------------------------------------------------------

/// DLLs do XInput, da mais nova para a mais antiga: a `1_4` vem no Windows 8
/// em diante; a `9_1_0`, no 7.
const XINPUT_DLLS: [&str; 2] = ["xinput1_4.dll", "xinput9_1_0.dll"];

/// `XInputGetState`, carregado na primeira leitura. Nulo se a carga falhou.
static GET_STATE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static LOAD_TRIED: AtomicBool = AtomicBool::new(false);

/// `XInputGetState` num slot vazio chega a custar milissegundos: um slot
/// desconectado so e consultado de novo depois de tantas leituras (~1 s a 60
/// frames por segundo).
const RETRY_UPDATES: u32 = 60;

/// Leituras que faltam para consultar cada slot desconectado de novo.
static RETRY_IN: [AtomicU32; GAMEPAD_COUNT] = [const { AtomicU32::new(0) }; GAMEPAD_COUNT];

const BUTTONS: [(GamepadButton, ffi::WORD); GAMEPAD_BUTTON_COUNT] = [
    (GamepadButton::South, ffi::XINPUT_GAMEPAD_A),
    (GamepadButton::East, ffi::XINPUT_GAMEPAD_B),
    (GamepadButton::West, ffi::XINPUT_GAMEPAD_X),
    (GamepadButton::North, ffi::XINPUT_GAMEPAD_Y),
    (GamepadButton::LeftShoulder, ffi::XINPUT_GAMEPAD_LEFT_SHOULDER),
    (GamepadButton::RightShoulder, ffi::XINPUT_GAMEPAD_RIGHT_SHOULDER),
    (GamepadButton::LeftStick, ffi::XINPUT_GAMEPAD_LEFT_THUMB),
    (GamepadButton::RightStick, ffi::XINPUT_GAMEPAD_RIGHT_THUMB),
    (GamepadButton::Back, ffi::XINPUT_GAMEPAD_BACK),
    (GamepadButton::Start, ffi::XINPUT_GAMEPAD_START),
    (GamepadButton::DpadUp, ffi::XINPUT_GAMEPAD_DPAD_UP),
    (GamepadButton::DpadDown, ffi::XINPUT_GAMEPAD_DPAD_DOWN),
    (GamepadButton::DpadLeft, ffi::XINPUT_GAMEPAD_DPAD_LEFT),
    (GamepadButton::DpadRight, ffi::XINPUT_GAMEPAD_DPAD_RIGHT),
];

/// Carrega o XInput na primeira chamada. A DLL nunca e descarregada: o
/// ponteiro guardado aponta para dentro dela ate o fim do processo.
fn xinput_get_state() -> Option<ffi::XInputGetState> {
    if !LOAD_TRIED.swap(true, Ordering::AcqRel) {
        for name in XINPUT_DLLS {
            let Some(library) = Win32::library_open(name) else {
                continue;
            };
            let symbol = Win32::library_symbol(&library, c"XInputGetState");
            if symbol.is_null() {
                unsafe { Win32::library_close(library) };
                continue;
            }
            GET_STATE.store(symbol, Ordering::Release);
            break;
        }
    }
    let symbol = GET_STATE.load(Ordering::Acquire);
    if symbol.is_null() {
        return None;
    }
    // O simbolo e o `XInputGetState` do XInput, com essa assinatura.
    Some(unsafe { mem::transmute::<*mut c_void, ffi::XInputGetState>(symbol) })
}

/// Zona morta por eixo; fora dela, o resto e reescalado para comecar em 0 na
/// borda da zona e chegar a 1 no fim do curso.
fn stick(value: ffi::SHORT, deadzone: ffi::SHORT) -> f32 {
    let (value, deadzone) = (value as i32, deadzone as i32);
    let axis = if value > deadzone {
        (value - deadzone) as f32 / (32767 - deadzone) as f32
    } else if value < -deadzone {
        (value + deadzone) as f32 / (32768 - deadzone) as f32
    } else {
        0.0
    };
    axis.clamp(-1.0, 1.0)
}

fn trigger(value: ffi::BYTE) -> f32 {
    let threshold = ffi::XINPUT_GAMEPAD_TRIGGER_THRESHOLD;
    if value <= threshold {
        return 0.0;
    }
    (value - threshold) as f32 / (255 - threshold) as f32
}

/// Desconectado: botoes que estavam apertados contam como soltos neste
/// frame, e o resto zera.
fn disconnect(pad: &mut Gamepad) {
    for button in pad.buttons.iter_mut() {
        button.set(false);
    }
    *pad = Gamepad { buttons: pad.buttons, ..Gamepad::default() };
}

fn read(pad: &mut Gamepad, state: &ffi::XINPUT_GAMEPAD) {
    pad.connected = true;
    for (button, mask) in BUTTONS {
        pad.buttons[button as usize].set(state.wButtons & mask != 0);
    }
    pad.left_x = stick(state.sThumbLX, ffi::XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE);
    pad.left_y = stick(state.sThumbLY, ffi::XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE);
    pad.right_x = stick(state.sThumbRX, ffi::XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE);
    pad.right_y = stick(state.sThumbRY, ffi::XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE);
    pad.left_trigger = trigger(state.bLeftTrigger);
    pad.right_trigger = trigger(state.bRightTrigger);
}

impl Gamepads for Win32 {
    /// Sem XInput no sistema, todos os slots ficam desconectados.
    fn gamepads_update(pads: &mut [Gamepad; GAMEPAD_COUNT]) {
        let get_state = xinput_get_state();
        for (index, pad) in pads.iter_mut().enumerate() {
            for button in pad.buttons.iter_mut() {
                button.begin_frame();
            }
            let Some(get_state) = get_state else {
                disconnect(pad);
                continue;
            };

            let retry = &RETRY_IN[index];
            if !pad.connected {
                let left = retry.load(Ordering::Relaxed);
                if left > 0 {
                    retry.store(left - 1, Ordering::Relaxed);
                    continue;
                }
            }

            let mut state: ffi::XINPUT_STATE = unsafe { mem::zeroed() };
            if unsafe { get_state(index as ffi::DWORD, &mut state) } != ffi::ERROR_SUCCESS {
                disconnect(pad);
                retry.store(RETRY_UPDATES, Ordering::Relaxed);
                continue;
            }
            read(pad, &state.Gamepad);
        }
    }
}
