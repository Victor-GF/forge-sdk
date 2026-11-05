use super::{Win32, ffi, input};
use crate::input::{Keyboard, Mouse, MouseButton};
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
    /// Input acumulado desde o ultimo `window_input`.
    keyboard: Keyboard,
    mouse: Mouse,
}

/// Fechar so marca o pedido: destruir a janela e decisao de quem chama.
///
/// O estado e acessado por ponteiro, em trechos curtos: algumas chamadas ao
/// user32 (`ReleaseCapture`, `DefWindowProcA`) mandam outra mensagem e
/// reentram aqui antes de voltar, entao nenhuma referencia ao estado pode
/// estar viva durante elas.
unsafe extern "system" fn window_proc(
    hwnd: ffi::HWND,
    msg: ffi::UINT,
    wparam: ffi::WPARAM,
    lparam: ffi::LPARAM,
) -> ffi::LRESULT {
    unsafe {
        let state = ffi::GetWindowLongPtrA(hwnd, ffi::GWLP_USERDATA) as *mut WindowState;
        if state.is_null() {
            // Mensagens de dentro do `CreateWindowExA`, antes do estado existir.
            return ffi::DefWindowProcA(hwnd, msg, wparam, lparam);
        }
        match msg {
            ffi::WM_CLOSE => {
                (*state).close_requested = true;
                return 0;
            }

            ffi::WM_KEYDOWN | ffi::WM_KEYUP | ffi::WM_SYSKEYDOWN | ffi::WM_SYSKEYUP => {
                let down = msg == ffi::WM_KEYDOWN || msg == ffi::WM_SYSKEYDOWN;
                let scancode = ((lparam >> 16) & 0xFF) as u8;
                let extended = (lparam >> 24) & 1 != 0;
                if let Some(key) = input::key_from_scancode(scancode, extended) {
                    (*state).keyboard.keys[key as usize].set(down);
                }
                // Das teclas de sistema, so Alt+F4 vai para o `DefWindowProcA`,
                // para continuar fechando a janela. Alt e F10 sozinhos ativariam
                // o menu da janela, que segura as proximas teclas.
                if msg == ffi::WM_SYSKEYDOWN && wparam == ffi::VK_F4 {
                    return ffi::DefWindowProcA(hwnd, msg, wparam, lparam);
                }
                return 0;
            }
            // Sem isso, Alt+letra faz o Windows bipar procurando um menu.
            ffi::WM_SYSCHAR => return 0,

            ffi::WM_MOUSEMOVE => {
                (*state).mouse.x = low_i16(lparam as usize) as i32;
                (*state).mouse.y = high_i16(lparam as usize) as i32;
                return 0;
            }
            ffi::WM_MOUSEWHEEL => {
                (*state).mouse.wheel += high_i16(wparam) as f32 / ffi::WHEEL_DELTA as f32;
                return 0;
            }
            ffi::WM_LBUTTONDOWN | ffi::WM_LBUTTONUP => {
                mouse_button(state, MouseButton::Left, msg == ffi::WM_LBUTTONDOWN);
                return 0;
            }
            ffi::WM_RBUTTONDOWN | ffi::WM_RBUTTONUP => {
                mouse_button(state, MouseButton::Right, msg == ffi::WM_RBUTTONDOWN);
                return 0;
            }
            ffi::WM_MBUTTONDOWN | ffi::WM_MBUTTONUP => {
                mouse_button(state, MouseButton::Middle, msg == ffi::WM_MBUTTONDOWN);
                return 0;
            }
            ffi::WM_XBUTTONDOWN | ffi::WM_XBUTTONUP => {
                let button = if (wparam >> 16) as u16 == ffi::XBUTTON1 { MouseButton::X1 } else { MouseButton::X2 };
                mouse_button(state, button, msg == ffi::WM_XBUTTONDOWN);
                // `WM_XBUTTON*` e a excecao: tratado, devolve TRUE.
                return 1;
            }

            // O sistema precisa do `DefWindowProcA` depois para limpar a
            // mensagem: sem `return`.
            ffi::WM_INPUT => read_raw_input(state, lparam),

            ffi::WM_KILLFOCUS => {
                for key in (*state).keyboard.keys.iter_mut() {
                    key.set(false);
                }
                for button in (*state).mouse.buttons.iter_mut() {
                    button.set(false);
                }
                ffi::ReleaseCapture();
            }
            _ => {}
        }
        ffi::DefWindowProcA(hwnd, msg, wparam, lparam)
    }
}

/// `LOWORD` com sinal: coordenadas podem ser negativas com o mouse capturado.
#[inline]
fn low_i16(value: usize) -> i16 {
    value as u16 as i16
}

#[inline]
fn high_i16(value: usize) -> i16 {
    (value >> 16) as u16 as i16
}

/// Com algum botao apertado, a janela captura o mouse: soltar o botao fora
/// dela tambem chega aqui, e o botao nao fica preso.
unsafe fn mouse_button(state: *mut WindowState, button: MouseButton, down: bool) {
    unsafe {
        (*state).mouse.buttons[button as usize].set(down);
        let any_down = (*state).mouse.buttons.iter().any(|button| button.down);
        if any_down {
            ffi::SetCapture((*state).hwnd);
        } else {
            ffi::ReleaseCapture();
        }
    }
}

/// Soma o movimento relativo do mouse. Posicao absoluta (area de trabalho
/// remota, mesa digitalizadora) nao e movimento e fica de fora.
unsafe fn read_raw_input(state: *mut WindowState, lparam: ffi::LPARAM) {
    unsafe {
        let mut raw: ffi::RAWINPUT = mem::zeroed();
        let mut size = mem::size_of::<ffi::RAWINPUT>() as ffi::UINT;
        let read = ffi::GetRawInputData(
            lparam as ffi::HRAWINPUT,
            ffi::RID_INPUT,
            (&raw mut raw).cast(),
            &mut size,
            mem::size_of::<ffi::RAWINPUTHEADER>() as ffi::UINT,
        );
        if read == u32::MAX || raw.header.dwType != ffi::RIM_TYPEMOUSE {
            return;
        }
        if raw.mouse.usFlags & ffi::MOUSE_MOVE_ABSOLUTE != 0 {
            return;
        }
        (*state).mouse.delta_x += raw.mouse.lLastX;
        (*state).mouse.delta_y += raw.mouse.lLastY;
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

            state.write(WindowState {
                hwnd,
                close_requested: false,
                keyboard: Keyboard::default(),
                mouse: Mouse::default(),
            });
            ffi::SetWindowLongPtrA(hwnd, ffi::GWLP_USERDATA, state as ffi::LONG_PTR);

            // Movimento cru do mouse, em `WM_INPUT`. Se o registro falhar, o
            // resto do mouse funciona; so `delta_x`/`delta_y` ficam zerados.
            let device = ffi::RAWINPUTDEVICE {
                usUsagePage: ffi::HID_USAGE_PAGE_GENERIC,
                usUsage: ffi::HID_USAGE_GENERIC_MOUSE,
                dwFlags: 0,
                hwndTarget: hwnd,
            };
            ffi::RegisterRawInputDevices(&device, 1, mem::size_of::<ffi::RAWINPUTDEVICE>() as ffi::UINT);

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

    fn window_input(window: &WindowHandle, keyboard: &mut Keyboard, mouse: &mut Mouse) {
        let state = unsafe { &mut *state(window) };
        *keyboard = state.keyboard;
        *mouse = state.mouse;

        for key in state.keyboard.keys.iter_mut() {
            key.begin_frame();
        }
        for button in state.mouse.buttons.iter_mut() {
            button.begin_frame();
        }
        state.mouse.delta_x = 0;
        state.mouse.delta_y = 0;
        state.mouse.wheel = 0.0;
    }
}
