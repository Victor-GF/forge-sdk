use super::{Win32, ffi};
use crate::memory::Memory;
use crate::window::{Window, WindowHandle};
use core::ffi::CStr;
use core::{mem, ptr};

const CLASS_NAME: &CStr = c"forge_window";
const STYLE: ffi::DWORD = ffi::WS_OVERLAPPEDWINDOW;

/// Estado de uma janela, no heap. `WindowHandle` aponta para ca e o HWND
/// guarda o mesmo ponteiro em `GWLP_USERDATA`, para `window_proc` achar.
struct WindowState {
    hwnd: ffi::HWND,
    close_requested: bool,
}

/// Fechar so marca o pedido: destruir a janela e decisao de quem chama.
unsafe extern "system" fn window_proc(
    hwnd: ffi::HWND,
    msg: ffi::UINT,
    wparam: ffi::WPARAM,
    lparam: ffi::LPARAM,
) -> ffi::LRESULT {
    unsafe {
        let state = ffi::GetWindowLongPtrA(hwnd, ffi::GWLP_USERDATA) as *mut WindowState;
        if msg == ffi::WM_CLOSE && !state.is_null() {
            (*state).close_requested = true;
            return 0;
        }
        ffi::DefWindowProcA(hwnd, msg, wparam, lparam)
    }
}

#[inline]
fn state(window: &WindowHandle) -> *mut WindowState {
    window.0.cast()
}

impl Window for Win32 {
    /// A classe e registrada a cada chamada: a partir da segunda o registro
    /// falha por ja existir, o que e o esperado. Se falhar de verdade, o
    /// `CreateWindowExA` falha junto.
    fn window_create(title: &CStr, width: u32, height: u32) -> Option<WindowHandle> {
        unsafe {
            let instance = ffi::GetModuleHandleA(ptr::null());

            let class = ffi::WNDCLASSEXA {
                cbSize: mem::size_of::<ffi::WNDCLASSEXA>() as ffi::UINT,
                style: ffi::CS_HREDRAW | ffi::CS_VREDRAW,
                lpfnWndProc: window_proc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: ptr::null_mut(),
                hCursor: ffi::LoadCursorA(ptr::null_mut(), ffi::IDC_ARROW),
                hbrBackground: ptr::null_mut(),
                lpszMenuName: ptr::null(),
                lpszClassName: CLASS_NAME.as_ptr(),
                hIconSm: ptr::null_mut(),
            };
            ffi::RegisterClassExA(&class);

            let mut rect = ffi::RECT { left: 0, top: 0, right: width as ffi::LONG, bottom: height as ffi::LONG };
            ffi::AdjustWindowRect(&mut rect, STYLE, 0);

            let state = Win32::heap_alloc(mem::size_of::<WindowState>()) as *mut WindowState;
            if state.is_null() {
                return None;
            }

            let hwnd = ffi::CreateWindowExA(
                0,
                CLASS_NAME.as_ptr(),
                title.as_ptr(),
                STYLE,
                ffi::CW_USEDEFAULT,
                ffi::CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null_mut(),
            );
            if hwnd.is_null() {
                Win32::heap_free(state.cast());
                return None;
            }

            state.write(WindowState { hwnd, close_requested: false });
            ffi::SetWindowLongPtrA(hwnd, ffi::GWLP_USERDATA, state as ffi::LONG_PTR);
            ffi::ShowWindow(hwnd, ffi::SW_SHOW);

            Some(WindowHandle(state.cast()))
        }
    }

    fn window_destroy(window: WindowHandle) {
        let state = state(&window);
        unsafe {
            ffi::DestroyWindow((*state).hwnd);
            Win32::heap_free(state.cast());
        }
    }

    fn window_pump_events() {
        unsafe {
            let mut msg: ffi::MSG = mem::zeroed();
            while ffi::PeekMessageA(&mut msg, ptr::null_mut(), 0, 0, ffi::PM_REMOVE) != 0 {
                ffi::TranslateMessage(&msg);
                ffi::DispatchMessageA(&msg);
            }
        }
    }

    #[inline]
    fn window_close_requested(window: &WindowHandle) -> bool {
        unsafe { (*state(window)).close_requested }
    }

    fn window_size(window: &WindowHandle) -> (u32, u32) {
        unsafe {
            let mut rect: ffi::RECT = mem::zeroed();
            ffi::GetClientRect((*state(window)).hwnd, &mut rect);
            ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32)
        }
    }
}
