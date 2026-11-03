use super::winnt::LONG;
use core::ffi::c_void;
use forge_core::layout;

pub type HWND = *mut c_void;
pub type HICON = *mut c_void;
pub type HCURSOR = HICON;
pub type HBRUSH = *mut c_void;
pub type HMENU = *mut c_void;

layout!(HWND, size = 8, align = 8);

#[repr(C)]
pub struct POINT {
    pub x: LONG,
    pub y: LONG,
}

layout!(POINT, size = 8, align = 4);

#[repr(C)]
pub struct RECT {
    pub left: LONG,
    pub top: LONG,
    pub right: LONG,
    pub bottom: LONG,
}

layout!(RECT, size = 16, align = 4);
