//! XInput nao e linkado: a DLL e carregada em runtime (`library_open`),
//! porque o nome dela muda por versao do Windows. Por isso as funcoes aqui
//! sao tipos de ponteiro, nao `extern`.

use super::minwindef::{BYTE, DWORD, WORD};
use super::winnt::SHORT;
use forge_core::layout;

#[repr(C)]
pub struct XINPUT_GAMEPAD {
    pub wButtons: WORD,
    pub bLeftTrigger: BYTE,
    pub bRightTrigger: BYTE,
    pub sThumbLX: SHORT,
    pub sThumbLY: SHORT,
    pub sThumbRX: SHORT,
    pub sThumbRY: SHORT,
}

layout!(XINPUT_GAMEPAD, size = 12, align = 2);

#[repr(C)]
pub struct XINPUT_STATE {
    pub dwPacketNumber: DWORD,
    pub Gamepad: XINPUT_GAMEPAD,
}

layout!(XINPUT_STATE, size = 16, align = 4);

pub type XInputGetState = unsafe extern "system" fn(dwUserIndex: DWORD, pState: *mut XINPUT_STATE) -> DWORD;

pub const XUSER_MAX_COUNT: DWORD = 4;

pub const XINPUT_GAMEPAD_DPAD_UP: WORD = 0x0001;
pub const XINPUT_GAMEPAD_DPAD_DOWN: WORD = 0x0002;
pub const XINPUT_GAMEPAD_DPAD_LEFT: WORD = 0x0004;
pub const XINPUT_GAMEPAD_DPAD_RIGHT: WORD = 0x0008;
pub const XINPUT_GAMEPAD_START: WORD = 0x0010;
pub const XINPUT_GAMEPAD_BACK: WORD = 0x0020;
pub const XINPUT_GAMEPAD_LEFT_THUMB: WORD = 0x0040;
pub const XINPUT_GAMEPAD_RIGHT_THUMB: WORD = 0x0080;
pub const XINPUT_GAMEPAD_LEFT_SHOULDER: WORD = 0x0100;
pub const XINPUT_GAMEPAD_RIGHT_SHOULDER: WORD = 0x0200;
pub const XINPUT_GAMEPAD_A: WORD = 0x1000;
pub const XINPUT_GAMEPAD_B: WORD = 0x2000;
pub const XINPUT_GAMEPAD_X: WORD = 0x4000;
pub const XINPUT_GAMEPAD_Y: WORD = 0x8000;

pub const XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE: SHORT = 7849;
pub const XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE: SHORT = 8689;
pub const XINPUT_GAMEPAD_TRIGGER_THRESHOLD: BYTE = 30;
