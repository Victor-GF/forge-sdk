use std::{
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, Ordering},
};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, HDC, SRCCOPY, StretchDIBits},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        AdjustWindowRect, CS_OWNDC, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect,
        IDC_ARROW, LoadCursorW, MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SetWindowTextW, TranslateMessage,
        WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        WS_VISIBLE,
    },
};

use super::Input;

static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE | WM_DESTROY => {
            CLOSE_REQUESTED.store(true, Ordering::Relaxed);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub struct Window {
    hwnd: HWND,
    hdc: HDC,
}

impl Window {
    /// Opens a window whose client area is `width` x `height`.
    pub fn open(title: &str, width: usize, height: usize) -> Result<Window, String> {
        let class_name = wide("forja_window");
        let title = wide(title);
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = WNDCLASSW {
                style: CS_OWNDC,
                lpfnWndProc: Some(window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: null_mut(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&class); // fails harmlessly if already registered
            let mut rect = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
            AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, 0);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW failed".into());
            }
            CLOSE_REQUESTED.store(false, Ordering::Relaxed);
            Ok(Window { hwnd, hdc: GetDC(hwnd) })
        }
    }

    /// Processes pending OS messages and updates `input`. Returns false once the window is closed.
    pub fn pump(&mut self, input: &mut Input) -> bool {
        input.pressed = [false; 256];
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                match msg.message {
                    WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                        let key = (msg.wParam & 0xff) as usize;
                        let down = msg.message == WM_KEYDOWN || msg.message == WM_SYSKEYDOWN;
                        let was_down = (msg.lParam >> 30) & 1 != 0;
                        if down && !was_down {
                            input.pressed[key] = true;
                        }
                        input.down[key] = down;
                    }
                    _ => {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        }
        !CLOSE_REQUESTED.load(Ordering::Relaxed)
    }

    /// Blits 0x00RRGGBB pixels, stretched to the client area.
    pub fn present(&mut self, pixels: &[u32], width: usize, height: usize) {
        assert_eq!(pixels.len(), width * height);
        unsafe {
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut client);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // negative: top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            StretchDIBits(
                self.hdc,
                0,
                0,
                client.right - client.left,
                client.bottom - client.top,
                0,
                0,
                width as i32,
                height as i32,
                pixels.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }

    pub fn set_title(&mut self, title: &str) {
        let title = wide(title);
        unsafe {
            SetWindowTextW(self.hwnd, title.as_ptr());
        }
    }
}
