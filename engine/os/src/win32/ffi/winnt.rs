use super::minwindef::DWORD;
use core::ffi::{c_char, c_void};
use forge_core::layout;

pub type LONG = i32;
pub type SHORT = i16;
pub type LONGLONG = i64;
pub type HRESULT = LONG;
pub type WCHAR = u16;
pub type HANDLE = *mut c_void;
pub type LPCSTR = *const c_char;
pub type LPCWSTR = *const WCHAR;
pub type LPWSTR = *mut WCHAR;
pub type PWSTR = *mut WCHAR;

layout!(HANDLE, size = 8, align = 8);

/// No SDK e uma union `QuadPart` / `{ LowPart, HighPart }`, resto da era de
/// 32 bits; aqui fica so `QuadPart`, com o mesmo layout.
#[repr(C)]
pub struct LARGE_INTEGER {
    pub QuadPart: LONGLONG,
}

layout!(LARGE_INTEGER, size = 8, align = 8);

pub type PLARGE_INTEGER = *mut LARGE_INTEGER;

pub const GENERIC_READ: DWORD = 0x8000_0000;
pub const GENERIC_WRITE: DWORD = 0x4000_0000;

pub const FILE_SHARE_READ: DWORD = 0x0000_0001;
pub const FILE_SHARE_WRITE: DWORD = 0x0000_0002;
pub const FILE_SHARE_DELETE: DWORD = 0x0000_0004;

pub const FILE_ATTRIBUTE_DIRECTORY: DWORD = 0x0000_0010;
pub const FILE_ATTRIBUTE_NORMAL: DWORD = 0x0000_0080;

pub const MEM_COMMIT: DWORD = 0x0000_1000;
pub const MEM_RESERVE: DWORD = 0x0000_2000;
pub const MEM_DECOMMIT: DWORD = 0x0000_4000;
pub const MEM_RELEASE: DWORD = 0x0000_8000;

pub const PAGE_NOACCESS: DWORD = 0x01;
pub const PAGE_READONLY: DWORD = 0x02;
pub const PAGE_READWRITE: DWORD = 0x04;

pub const HEAP_ZERO_MEMORY: DWORD = 0x0000_0008;

pub const TIMER_ALL_ACCESS: DWORD = 0x001F_0003;
