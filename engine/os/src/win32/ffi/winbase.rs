use super::basetsd::DWORD_PTR;
use super::minwindef::{BOOL, DWORD, INT};
use super::winnt::{HANDLE, LPCWSTR};

pub const INFINITE: DWORD = 0xFFFF_FFFF;

pub const STD_OUTPUT_HANDLE: DWORD = -11i32 as DWORD;

pub const MOVEFILE_REPLACE_EXISTING: DWORD = 0x0000_0001;
/// So volta depois que a troca esta no disco.
pub const MOVEFILE_WRITE_THROUGH: DWORD = 0x0000_0008;

/// `dwCreationFlags` de `CreateThread`: nasce parada, ate `ResumeThread`.
pub const CREATE_SUSPENDED: DWORD = 0x0000_0004;
/// `dwStackSize` de `CreateThread` e a reserva, nao o commit inicial.
pub const STACK_SIZE_PARAM_IS_A_RESERVATION: DWORD = 0x0001_0000;

pub const THREAD_PRIORITY_BELOW_NORMAL: INT = -1;
pub const THREAD_PRIORITY_NORMAL: INT = 0;
pub const THREAD_PRIORITY_ABOVE_NORMAL: INT = 1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn MoveFileExW(lpExistingFileName: LPCWSTR, lpNewFileName: LPCWSTR, dwFlags: DWORD) -> BOOL;
    pub fn SetThreadAffinityMask(hThread: HANDLE, dwThreadAffinityMask: DWORD_PTR) -> DWORD_PTR;
    pub fn CopyFileW(lpExistingFileName: LPCWSTR, lpNewFileName: LPCWSTR, bFailIfExists: BOOL) -> BOOL;
}
