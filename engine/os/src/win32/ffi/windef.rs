use super::winnt::LONG;
use core::ffi::c_void;
use forge_core::layout;

pub type HWND = *mut c_void;
pub type HICON = *mut c_void;
pub type HCURSOR = HICON;
pub type HBRUSH = *mut c_void;
pub type HMENU = *mut c_void;
pub type HMONITOR = *mut c_void;
pub type HDC = *mut c_void;

/// `DECLARE_HANDLE` no SDK; os valores predefinidos sao negativos.
pub type DPI_AWARENESS_CONTEXT = *mut c_void;

pub const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: DPI_AWARENESS_CONTEXT = -4isize as DPI_AWARENESS_CONTEXT;

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
