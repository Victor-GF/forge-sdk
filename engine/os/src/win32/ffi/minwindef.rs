use super::basetsd::{LONG_PTR, UINT_PTR};
use super::winnt::LONG;
use core::ffi::c_void;
use forge_core::layout;

pub type BOOL = i32;
pub type WORD = u16;
pub type DWORD = u32;
pub type UINT = u32;
pub type INT = i32;
pub type ATOM = WORD;
pub type LPVOID = *mut c_void;
pub type LPDWORD = *mut DWORD;
pub type LPLONG = *mut LONG;

pub type WPARAM = UINT_PTR;
pub type LPARAM = LONG_PTR;
pub type LRESULT = LONG_PTR;

pub type HINSTANCE = *mut c_void;
pub type HMODULE = HINSTANCE;

layout!(HINSTANCE, size = 8, align = 8);
