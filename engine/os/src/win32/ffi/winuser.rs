use super::basetsd::LONG_PTR;
use super::minwindef::{ATOM, BOOL, DWORD, HINSTANCE, INT, LPARAM, LPVOID, LRESULT, UINT, ULONG, USHORT, WPARAM};
use super::windef::{HBRUSH, HCURSOR, HICON, HMENU, HWND, POINT, RECT};
use super::winnt::{HANDLE, LONG, LPCSTR};
use core::ffi::c_void;
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

pub const WM_SETFOCUS: UINT = 0x0007;
pub const WM_KILLFOCUS: UINT = 0x0008;
pub const WM_CLOSE: UINT = 0x0010;
pub const WM_INPUT: UINT = 0x00FF;
pub const WM_KEYDOWN: UINT = 0x0100;
pub const WM_KEYUP: UINT = 0x0101;
pub const WM_SYSKEYDOWN: UINT = 0x0104;
pub const WM_SYSKEYUP: UINT = 0x0105;
pub const WM_SYSCHAR: UINT = 0x0106;
pub const WM_MOUSEMOVE: UINT = 0x0200;
pub const WM_LBUTTONDOWN: UINT = 0x0201;
pub const WM_LBUTTONUP: UINT = 0x0202;
pub const WM_RBUTTONDOWN: UINT = 0x0204;
pub const WM_RBUTTONUP: UINT = 0x0205;
pub const WM_MBUTTONDOWN: UINT = 0x0207;
pub const WM_MBUTTONUP: UINT = 0x0208;
pub const WM_MOUSEWHEEL: UINT = 0x020A;
pub const WM_XBUTTONDOWN: UINT = 0x020B;
pub const WM_XBUTTONUP: UINT = 0x020C;

pub const VK_F4: WPARAM = 0x73;

pub const WHEEL_DELTA: i16 = 120;

/// `HIWORD(wParam)` de `WM_XBUTTON*`.
pub const XBUTTON1: u16 = 0x0001;
pub const XBUTTON2: u16 = 0x0002;

// ---- Raw input ----------------------------------------------------------

pub type HRAWINPUT = *mut c_void;

pub const RID_INPUT: UINT = 0x1000_0003;
pub const RIM_TYPEMOUSE: DWORD = 0;
/// `RAWMOUSE::usFlags`: posicao absoluta (area remota, tablet) em vez de
/// movimento relativo.
pub const MOUSE_MOVE_ABSOLUTE: USHORT = 0x01;

#[repr(C)]
pub struct RAWINPUTDEVICE {
    pub usUsagePage: USHORT,
    pub usUsage: USHORT,
    pub dwFlags: DWORD,
    pub hwndTarget: HWND,
}

layout!(RAWINPUTDEVICE, size = 16, align = 8);

#[repr(C)]
pub struct RAWINPUTHEADER {
    pub dwType: DWORD,
    pub dwSize: DWORD,
    pub hDevice: HANDLE,
    pub wParam: WPARAM,
}

layout!(RAWINPUTHEADER, size = 24, align = 8);

/// No SDK, `usButtonFlags`/`usButtonData` estao numa union com
/// `ulButtons: ULONG`, alinhada a 4: dai `_padding` depois de `usFlags`.
#[repr(C)]
pub struct RAWMOUSE {
    pub usFlags: USHORT,
    pub _padding: USHORT,
    pub usButtonFlags: USHORT,
    pub usButtonData: USHORT,
    pub ulRawButtons: ULONG,
    pub lLastX: LONG,
    pub lLastY: LONG,
    pub ulExtraInformation: ULONG,
}

layout!(RAWMOUSE, size = 24, align = 4);

/// No SDK, `data` e uma union de mouse, teclado e HID. So registramos mouse:
/// aqui fica so `mouse`, que e o maior membro de tamanho fixo.
#[repr(C)]
pub struct RAWINPUT {
    pub header: RAWINPUTHEADER,
    pub mouse: RAWMOUSE,
}

layout!(RAWINPUT, size = 48, align = 8);

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

    pub fn SetCapture(hWnd: HWND) -> HWND;
    pub fn ReleaseCapture() -> BOOL;

    pub fn RegisterRawInputDevices(pRawInputDevices: *const RAWINPUTDEVICE, uiNumDevices: UINT, cbSize: UINT) -> BOOL;
    pub fn GetRawInputData(
        hRawInput: HRAWINPUT,
        uiCommand: UINT,
        pData: LPVOID,
        pcbSize: *mut UINT,
        cbSizeHeader: UINT,
    ) -> UINT;
}
