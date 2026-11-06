use super::basetsd::{LONG_PTR, UINT_PTR};
use super::winnt::LONG;
use core::ffi::c_void;
use forge_core::layout;

pub type BYTE = u8;
pub type BOOL = i32;
pub type BOOLEAN = BYTE;
pub type WORD = u16;
pub type USHORT = u16;
pub type ULONG = u32;
pub type DWORD = u32;
pub type UINT32 = u32;
pub type UINT = u32;
pub type INT = i32;
pub type INT8 = i8;
pub type INT16 = i16;
pub type INT32 = i32;
pub type INT64 = i64;
pub type ATOM = WORD;
pub type LPVOID = *mut c_void;
pub type LPCVOID = *const c_void;
pub type LPDWORD = *mut DWORD;
pub type LPLONG = *mut LONG;

pub const MAX_PATH: usize = 260;

#[repr(C)]
pub struct FILETIME {
    pub dwLowDateTime: DWORD,
    pub dwHighDateTime: DWORD,
}

layout!(FILETIME, size = 8, align = 4);

pub type WPARAM = UINT_PTR;
pub type LPARAM = LONG_PTR;
pub type LRESULT = LONG_PTR;

pub type HINSTANCE = *mut c_void;
pub type HMODULE = HINSTANCE;

/// No SDK e um ponteiro de funcao que pode ser nulo; aqui, ponteiro cru com o
/// mesmo layout. Quem usa converte para a assinatura certa.
pub type FARPROC = *mut c_void;

layout!(HINSTANCE, size = 8, align = 8);
