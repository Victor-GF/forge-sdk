use super::minwindef::DWORD;
use super::winnt::HRESULT;

pub const ERROR_SUCCESS: DWORD = 0;
pub const ERROR_HANDLE_EOF: DWORD = 38;
pub const ERROR_INSUFFICIENT_BUFFER: DWORD = 122;

pub const S_OK: HRESULT = 0;
pub const S_FALSE: HRESULT = 1;
/// COM ja foi iniciado nesta thread em outro modelo (STA). Continua usavel.
pub const RPC_E_CHANGED_MODE: HRESULT = 0x8001_0106_u32 as HRESULT;
