use super::basetsd::LONG_PTR;
use super::minwindef::{ATOM, BOOL, DWORD, HINSTANCE, INT, LPARAM, LPVOID, LRESULT, UINT, ULONG, USHORT, WPARAM};
use super::windef::{DPI_AWARENESS_CONTEXT, HBRUSH, HCURSOR, HDC, HICON, HMENU, HMONITOR, HWND, POINT, RECT};
use super::winnt::{HANDLE, LONG, LPCWSTR};
use core::ffi::c_void;
use forge_core::layout;

pub type WNDPROC = unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: UINT,
    pub style: UINT,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: INT,
    pub cbWndExtra: INT,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: LPCWSTR,
    pub lpszClassName: LPCWSTR,
    pub hIconSm: HICON,
}

layout!(WNDCLASSEXW, size = 80, align = 8);

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
pub const WS_POPUP: DWORD = 0x8000_0000;

pub const GWL_STYLE: INT = -16;

pub const SWP_NOSIZE: UINT = 0x0001;
pub const SWP_NOMOVE: UINT = 0x0002;
pub const SWP_NOZORDER: UINT = 0x0004;
pub const SWP_NOACTIVATE: UINT = 0x0010;
pub const SWP_FRAMECHANGED: UINT = 0x0020;
pub const SWP_NOOWNERZORDER: UINT = 0x0200;

pub const MONITOR_DEFAULTTONEAREST: DWORD = 0x0000_0002;

pub const USER_DEFAULT_SCREEN_DPI: UINT = 96;

#[repr(C)]
pub struct MONITORINFO {
    pub cbSize: DWORD,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: DWORD,
}

layout!(MONITORINFO, size = 40, align = 4);

#[repr(C)]
pub struct WINDOWPLACEMENT {
    pub length: UINT,
    pub flags: UINT,
    pub showCmd: UINT,
    pub ptMinPosition: POINT,
    pub ptMaxPosition: POINT,
    pub rcNormalPosition: RECT,
}

layout!(WINDOWPLACEMENT, size = 44, align = 4);

pub const CW_USEDEFAULT: INT = 0x8000_0000_u32 as INT;

pub const SW_SHOW: INT = 5;

pub const PM_REMOVE: UINT = 0x0001;

pub const WM_SETFOCUS: UINT = 0x0007;
pub const WM_KILLFOCUS: UINT = 0x0008;
pub const WM_CLOSE: UINT = 0x0010;
pub const WM_QUERYENDSESSION: UINT = 0x0011;
pub const WM_ENDSESSION: UINT = 0x0016;
pub const WM_POWERBROADCAST: UINT = 0x0218;
pub const WM_INPUT: UINT = 0x00FF;
pub const WM_DPICHANGED: UINT = 0x02E0;
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

/// `wParam` de `WM_POWERBROADCAST`.
pub const PBT_APMSUSPEND: WPARAM = 0x0004;
pub const PBT_APMRESUMESUSPEND: WPARAM = 0x0007;
pub const PBT_APMRESUMEAUTOMATIC: WPARAM = 0x0012;

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

/// `MAKEINTRESOURCEW(32512)`: o id vai no proprio valor do ponteiro.
pub const IDC_ARROW: LPCWSTR = 32512 as LPCWSTR;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn MessageBoxW(hWnd: HWND, lpText: LPCWSTR, lpCaption: LPCWSTR, uType: UINT) -> INT;

    pub fn RegisterClassExW(lpWndClass: *const WNDCLASSEXW) -> ATOM;
    pub fn CreateWindowExW(
        dwExStyle: DWORD,
        lpClassName: LPCWSTR,
        lpWindowName: LPCWSTR,
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
    pub fn DefWindowProcW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;

    pub fn PeekMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: UINT, wMsgFilterMax: UINT, wRemoveMsg: UINT) -> BOOL;
    pub fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;

    pub fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    pub fn AdjustWindowRect(lpRect: *mut RECT, dwStyle: DWORD, bMenu: BOOL) -> BOOL;

    pub fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: LPCWSTR) -> HCURSOR;

    pub fn SetWindowLongPtrW(hWnd: HWND, nIndex: INT, dwNewLong: LONG_PTR) -> LONG_PTR;
    pub fn GetWindowLongPtrW(hWnd: HWND, nIndex: INT) -> LONG_PTR;

    pub fn IsIconic(hWnd: HWND) -> BOOL;
    pub fn GetForegroundWindow() -> HWND;

    pub fn SetWindowPos(hWnd: HWND, hWndInsertAfter: HWND, X: INT, Y: INT, cx: INT, cy: INT, uFlags: UINT) -> BOOL;
    pub fn GetWindowPlacement(hWnd: HWND, lpwndpl: *mut WINDOWPLACEMENT) -> BOOL;
    pub fn SetWindowPlacement(hWnd: HWND, lpwndpl: *const WINDOWPLACEMENT) -> BOOL;

    pub fn MonitorFromWindow(hwnd: HWND, dwFlags: DWORD) -> HMONITOR;
    pub fn GetMonitorInfoW(hMonitor: HMONITOR, lpmi: *mut MONITORINFO) -> BOOL;

    /// Windows 10 1703 em diante.
    pub fn SetProcessDpiAwarenessContext(value: DPI_AWARENESS_CONTEXT) -> BOOL;
    /// Windows 10 1607 em diante.
    pub fn GetDpiForWindow(hwnd: HWND) -> UINT;
    /// Windows 10 1607 em diante.
    pub fn AdjustWindowRectExForDpi(lpRect: *mut RECT, dwStyle: DWORD, bMenu: BOOL, dwExStyle: DWORD, dpi: UINT) -> BOOL;

    pub fn GetDC(hWnd: HWND) -> HDC;
    pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> INT;

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
