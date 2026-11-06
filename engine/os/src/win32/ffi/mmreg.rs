use super::guiddef::GUID;
use super::minwindef::{DWORD, WORD};
use super::mmeapi::WAVEFORMATEX;
use forge_core::layout;

pub const WAVE_FORMAT_EXTENSIBLE: WORD = 0xFFFE;

pub const SPEAKER_FRONT_LEFT: DWORD = 0x1;
pub const SPEAKER_FRONT_RIGHT: DWORD = 0x2;

/// Empacotada como `WAVEFORMATEX`. No SDK, `Samples` e uma union de tres
/// `WORD`; aqui fica o membro que usamos (`wValidBitsPerSample`).
#[repr(C, packed)]
pub struct WAVEFORMATEXTENSIBLE {
    pub Format: WAVEFORMATEX,
    pub wValidBitsPerSample: WORD,
    pub dwChannelMask: DWORD,
    pub SubFormat: GUID,
}

layout!(WAVEFORMATEXTENSIBLE, size = 40, align = 1);
