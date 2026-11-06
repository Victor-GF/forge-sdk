use super::minwindef::DWORD;

/// Enum do SDK.
pub type AUDCLNT_SHAREMODE = i32;
pub const AUDCLNT_SHAREMODE_SHARED: AUDCLNT_SHAREMODE = 0;

/// Converte do formato pedido para o do mixer do sistema (taxa e canais).
pub const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: DWORD = 0x8000_0000;
/// Conversao de taxa com qualidade melhor que a padrao do `AUTOCONVERTPCM`.
pub const AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY: DWORD = 0x0800_0000;
