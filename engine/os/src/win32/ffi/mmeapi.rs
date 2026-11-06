use super::minwindef::{DWORD, WORD};
use forge_core::layout;

/// Empacotada (`pshpack1.h` no SDK): 18 bytes, sem preenchimento.
#[repr(C, packed)]
pub struct WAVEFORMATEX {
    pub wFormatTag: WORD,
    pub nChannels: WORD,
    pub nSamplesPerSec: DWORD,
    pub nAvgBytesPerSec: DWORD,
    pub nBlockAlign: WORD,
    pub wBitsPerSample: WORD,
    pub cbSize: WORD,
}

layout!(WAVEFORMATEX, size = 18, align = 1);
