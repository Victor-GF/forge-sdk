use super::minwindef::DWORD;
use core::ffi::{c_char, c_void};
use forge_core::layout;

pub type LONG = i32;
pub type LONGLONG = i64;
pub type WCHAR = u16;
pub type HANDLE = *mut c_void;
pub type LPCSTR = *const c_char;
pub type LPCWSTR = *const WCHAR;

layout!(HANDLE, size = 8, align = 8);

/// No SDK e uma union `QuadPart` / `{ LowPart, HighPart }`, resto da era de
/// 32 bits; aqui fica so `QuadPart`, com o mesmo layout.
#[repr(C)]
pub struct LARGE_INTEGER {
    pub QuadPart: LONGLONG,
}

layout!(LARGE_INTEGER, size = 8, align = 8);

pub const MEM_COMMIT: DWORD = 0x0000_1000;
pub const MEM_RESERVE: DWORD = 0x0000_2000;
pub const MEM_DECOMMIT: DWORD = 0x0000_4000;
pub const MEM_RELEASE: DWORD = 0x0000_8000;

pub const PAGE_NOACCESS: DWORD = 0x01;
pub const PAGE_READWRITE: DWORD = 0x04;

pub const HEAP_ZERO_MEMORY: DWORD = 0x0000_0008;
