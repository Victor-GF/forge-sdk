//! Backend de janela do Windows (Win32 direto).
//!
//! Sem estado global: a janela não guarda ponteiro para `Platform`; o input é lido direto da
//! fila de mensagens em `window_poll`, e tamanho/foco são consultados lá.
//!
//! Limitação conhecida: enquanto o usuário arrasta ou redimensiona a janela, o Windows roda um
//! loop modal próprio e o frame do jogo para até ele soltar.

use core::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, GetClientRect, GetForegroundWindow, IDC_ARROW, LoadCursorW,
    MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassExW, SC_KEYMENU, SW_SHOW,
    SetWindowTextW, ShowWindow, UnregisterClassW, WM_CLOSE, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW,
};

use crate::input::{Input, Key, MouseButton};
use crate::{PlatformError, TITLE_MAX, WindowDesc, WindowState};

const CLASS_NAME: &str = "forja_window";
const CLASS_NAME_MAX: usize = 32;

pub(crate) struct Window {
    hwnd: HWND,
    hinstance: HINSTANCE,
}

/// Converte `s` para UTF-16 terminado em 0 dentro de `buf`. `None` se não couber.
fn utf16_into<'b>(s: &str, buf: &'b mut [u16]) -> Option<&'b [u16]> {
    let mut n: usize = 0;
    for c in s.encode_utf16() {
        *buf.get_mut(n)? = c;
        n = n.checked_add(1)?;
    }
    *buf.get_mut(n)? = 0;
    buf.get(..=n)
}

pub(crate) fn window_create(desc: &WindowDesc) -> Result<Window, PlatformError> {
    let mut class_buf = [0u16; CLASS_NAME_MAX];
    let mut title_buf = [0u16; TITLE_MAX];
    let Some(class) = utf16_into(CLASS_NAME, &mut class_buf) else {
        return Err(PlatformError::RegisterClass);
    };
    let Some(title) = utf16_into(desc.title, &mut title_buf) else {
        return Err(PlatformError::TitleTooLong);
    };
    let width = i32::try_from(desc.width).unwrap_or(i32::MAX);
    let height = i32::try_from(desc.height).unwrap_or(i32::MAX);

    // SAFETY: chamadas Win32 com ponteiros para dados locais vivos durante a chamada;
    // `class` e `title` terminam em 0.
    unsafe {
        // Pixels físicos em telas com escala (pode falhar se já foi definido; tudo bem).
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinstance = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(BLACK_BRUSH),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: null_mut(),
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(PlatformError::RegisterClass);
        }

        let style = WS_OVERLAPPEDWINDOW;
        let mut r = RECT { left: 0, top: 0, right: width, bottom: height };
        AdjustWindowRectEx(&mut r, style, 0, 0);

        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            r.right.saturating_sub(r.left),
            r.bottom.saturating_sub(r.top),
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );
        if hwnd.is_null() {
            UnregisterClassW(class.as_ptr(), hinstance);
            return Err(PlatformError::CreateWindow);
        }
        ShowWindow(hwnd, SW_SHOW);
        Ok(Window { hwnd, hinstance })
    }
}

pub(crate) fn window_destroy(w: Window) {
    let mut class_buf = [0u16; CLASS_NAME_MAX];
    // SAFETY: `hwnd` veio de `window_create` e ainda é válido; `class` termina em 0.
    unsafe {
        DestroyWindow(w.hwnd);
        if let Some(class) = utf16_into(CLASS_NAME, &mut class_buf) {
            UnregisterClassW(class.as_ptr(), w.hinstance);
        }
    }
}

pub(crate) fn window_set_title(w: &Window, title: &str) {
    let mut buf = [0u16; TITLE_MAX];
    let Some(t) = utf16_into(title, &mut buf) else {
        return;
    };
    // SAFETY: `t` termina em 0 e vive durante a chamada.
    unsafe { SetWindowTextW(w.hwnd, t.as_ptr()) };
}

pub(crate) fn window_poll(w: &Window, input: &mut Input) -> WindowState {
    let mut closed = false;
    // SAFETY: MSG é uma struct C só com inteiros e ponteiros; zero é um valor válido.
    let mut msg: MSG = unsafe { core::mem::zeroed() };
    // SAFETY: `msg` é local e válido; hwnd nulo = todas as mensagens da thread.
    while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
        match msg.message {
            WM_QUIT => closed = true,
            WM_KEYDOWN | WM_SYSKEYDOWN => input.key_event(key_from_wparam(msg.wParam), true),
            WM_KEYUP | WM_SYSKEYUP => input.key_event(key_from_wparam(msg.wParam), false),
            WM_MOUSEMOVE => input.mouse_move(lparam_x(msg.lParam), lparam_y(msg.lParam)),
            WM_LBUTTONDOWN => input.mouse_button_event(MouseButton::Left, true),
            WM_LBUTTONUP => input.mouse_button_event(MouseButton::Left, false),
            WM_RBUTTONDOWN => input.mouse_button_event(MouseButton::Right, true),
            WM_RBUTTONUP => input.mouse_button_event(MouseButton::Right, false),
            WM_MBUTTONDOWN => input.mouse_button_event(MouseButton::Middle, true),
            WM_MBUTTONUP => input.mouse_button_event(MouseButton::Middle, false),
            WM_MOUSEWHEEL => input.wheel_event(wheel_from_wparam(msg.wParam)),
            _ => {}
        }
        // Sem TranslateMessage: ainda não há entrada de texto (WM_CHAR).
        // SAFETY: `msg` veio de PeekMessageW.
        unsafe { DispatchMessageW(&msg) };
    }

    // SAFETY: chamada sem ponteiros.
    let focused = unsafe { GetForegroundWindow() } == w.hwnd;
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: `r` é local e válido.
    unsafe { GetClientRect(w.hwnd, &mut r) };
    WindowState {
        closed,
        focused,
        width: u32::try_from(r.right.saturating_sub(r.left)).unwrap_or(0),
        height: u32::try_from(r.bottom.saturating_sub(r.top)).unwrap_or(0),
    }
}

/// Sem estado: fechar vira WM_QUIT na fila (lido em `window_poll`), e o menu do Alt/F10 é
/// bloqueado para não pausar o jogo num loop modal.
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            // SAFETY: chamada sem ponteiros.
            unsafe { PostQuitMessage(0) };
            0
        }
        WM_SYSCOMMAND if (wparam & 0xFFF0) == SC_KEYMENU as usize => 0,
        // SAFETY: repassa os argumentos recebidos do próprio Windows.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn key_from_wparam(w: WPARAM) -> Key {
    Key((w & 0xFF) as u8)
}

fn lparam_x(l: LPARAM) -> i32 {
    ((l as usize) & 0xFFFF) as u16 as i16 as i32
}

fn lparam_y(l: LPARAM) -> i32 {
    ((l as usize).wrapping_shr(16) & 0xFFFF) as u16 as i16 as i32
}

fn wheel_from_wparam(w: WPARAM) -> i32 {
    (w.wrapping_shr(16) & 0xFFFF) as u16 as i16 as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_mouse_coordinates() {
        // x = -5, y = 300 (mouse fora da janela à esquerda, com captura)
        let l = ((300usize << 16) | 0xFFFB) as LPARAM;
        assert_eq!(lparam_x(l), -5);
        assert_eq!(lparam_y(l), 300);
    }

    #[test]
    fn backward_wheel_is_negative() {
        let w = ((-120i16 as u16 as usize) << 16) as WPARAM;
        assert_eq!(wheel_from_wparam(w), -120);
    }

    #[test]
    fn utf16_is_null_terminated_and_respects_buffer() {
        let mut buf = [0xFFFFu16; 4];
        assert_eq!(utf16_into("ab", &mut buf), Some(&[0x61, 0x62, 0][..]));
        assert_eq!(utf16_into("abcd", &mut buf), None); // falta espaço para o 0
    }
}
