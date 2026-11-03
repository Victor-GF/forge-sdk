use super::basetsd::LONG_PTR;
use super::minwindef::{ATOM, BOOL, DWORD, HINSTANCE, INT, LPARAM, LPVOID, LRESULT, UINT, WPARAM};
use super::windef::{HBRUSH, HCURSOR, HICON, HMENU, HWND, POINT, RECT};
use super::winnt::LPCSTR;
use forge_core::layout;

pub type WNDPROC = unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
pub struct WNDCLASSEXA {
    pub cbSize: UINT,
    pub style: UINT,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: INT,
    pub cbWndExtra: INT,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: LPCSTR,
    pub lpszClassName: LPCSTR,
    pub hIconSm: HICON,
}

layout!(WNDCLASSEXA, size = 80, align = 8);

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: UINT,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: DWORD,
    pub pt: POINT,
}

layout!(MSG, size = 48, align = 8);

pub const MB_OK: UINT = 0x0000_0000;
pub const MB_ICONINFORMATION: UINT = 0x0000_0040;

pub const CS_VREDRAW: UINT = 0x0001;
pub const CS_HREDRAW: UINT = 0x0002;

pub const WS_OVERLAPPEDWINDOW: DWORD = 0x00CF_0000;

pub const CW_USEDEFAULT: INT = 0x8000_0000_u32 as INT;

pub const SW_SHOW: INT = 5;

pub const PM_REMOVE: UINT = 0x0001;

pub const WM_CLOSE: UINT = 0x0010;

pub const GWLP_USERDATA: INT = -21;

/// `MAKEINTRESOURCEA(32512)`: o id vai no proprio valor do ponteiro.
pub const IDC_ARROW: LPCSTR = 32512 as LPCSTR;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn MessageBoxA(hWnd: HWND, lpText: LPCSTR, lpCaption: LPCSTR, uType: UINT) -> INT;

    pub fn RegisterClassExA(lpWndClass: *const WNDCLASSEXA) -> ATOM;
    pub fn CreateWindowExA(
        dwExStyle: DWORD,
        lpClassName: LPCSTR,
        lpWindowName: LPCSTR,
        dwStyle: DWORD,
        X: INT,
        Y: INT,
        nWidth: INT,
        nHeight: INT,
        hWndParent: HWND,
        hMenu: HMENU,
        hInstance: HINSTANCE,
        lpParam: LPVOID,
    ) -> HWND;
    pub fn DestroyWindow(hWnd: HWND) -> BOOL;
    pub fn ShowWindow(hWnd: HWND, nCmdShow: INT) -> BOOL;
    pub fn DefWindowProcA(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;

    pub fn PeekMessageA(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: UINT, wMsgFilterMax: UINT, wRemoveMsg: UINT) -> BOOL;
    pub fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    pub fn DispatchMessageA(lpMsg: *const MSG) -> LRESULT;

    pub fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    pub fn AdjustWindowRect(lpRect: *mut RECT, dwStyle: DWORD, bMenu: BOOL) -> BOOL;

    pub fn LoadCursorA(hInstance: HINSTANCE, lpCursorName: LPCSTR) -> HCURSOR;

    pub fn SetWindowLongPtrA(hWnd: HWND, nIndex: INT, dwNewLong: LONG_PTR) -> LONG_PTR;
    pub fn GetWindowLongPtrA(hWnd: HWND, nIndex: INT) -> LONG_PTR;
}
