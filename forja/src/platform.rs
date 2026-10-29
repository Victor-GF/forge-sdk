//! Camada de plataforma Win32 escrita à mão: janela, teclado e apresentação de um
//! framebuffer de software via DIB section. Texto usa GDI direto sobre o DIB.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Mutex;

pub type HANDLE = *mut c_void;
type HWND = HANDLE;
type HDC = HANDLE;
type HINSTANCE = HANDLE;
type HGDIOBJ = HANDLE;
type WNDPROC = unsafe extern "system" fn(HWND, u32, usize, isize) -> isize;

#[repr(C)]
struct WNDCLASSEXW {
    cbSize: u32,
    style: u32,
    lpfnWndProc: WNDPROC,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HINSTANCE,
    hIcon: HANDLE,
    hCursor: HANDLE,
    hbrBackground: HANDLE,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
    hIconSm: HANDLE,
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: usize,
    lParam: isize,
    time: u32,
    pt: POINT,
    lPrivate: u32,
}

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

#[repr(C)]
struct BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER,
    bmiColors: [u32; 1],
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SYSTEMTIME {
    pub wYear: u16,
    pub wMonth: u16,
    pub wDayOfWeek: u16,
    pub wDay: u16,
    pub wHour: u16,
    pub wMinute: u16,
    pub wSecond: u16,
    pub wMilliseconds: u16,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32,
        parent: HWND, menu: HANDLE, inst: HINSTANCE, param: *mut c_void,
    ) -> HWND;
    fn DefWindowProcW(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn GetDC(hwnd: HWND) -> HDC;
    fn LoadCursorW(inst: HINSTANCE, name: *const u16) -> HANDLE;
    fn AdjustWindowRect(r: *mut RECT, style: u32, menu: i32) -> i32;
    fn SetWindowTextW(hwnd: HWND, text: *const u16) -> i32;
    pub fn MessageBoxW(hwnd: HWND, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn CreateDIBSection(hdc: HDC, bmi: *const BITMAPINFO, usage: u32, bits: *mut *mut c_void, section: HANDLE, offset: u32) -> HANDLE;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn BitBlt(dst: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32, rop: u32) -> i32;
    fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    fn SetTextColor(hdc: HDC, color: u32) -> u32;
    fn TextOutW(hdc: HDC, x: i32, y: i32, s: *const u16, n: i32) -> i32;
    fn CreateFontW(
        h: i32, w: i32, esc: i32, orient: i32, weight: i32, italic: u32, underline: u32, strike: u32,
        charset: u32, out_prec: u32, clip_prec: u32, quality: u32, pitch: u32, face: *const u16,
    ) -> HANDLE;
    fn GdiFlush() -> i32;
    fn DeleteObject(obj: HGDIOBJ) -> i32;
    fn DeleteDC(hdc: HDC) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    pub fn GetLocalTime(st: *mut SYSTEMTIME);
}

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(p: u32) -> u32;
}

const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_KILLFOCUS: u32 = 0x0008;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_QUIT: u32 = 0x0012;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_CHAR: u32 = 0x0102;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_EXITSIZEMOVE: u32 = 0x0232;

pub mod vk {
    pub const TAB: u8 = 0x09;
    pub const SHIFT: u8 = 0x10;
    pub const ESC: u8 = 0x1B;
    pub const SPACE: u8 = 0x20;
    pub const PGUP: u8 = 0x21;
    pub const PGDN: u8 = 0x22;
    pub const LEFT: u8 = 0x25;
    pub const UP: u8 = 0x26;
    pub const RIGHT: u8 = 0x27;
    pub const DOWN: u8 = 0x28;
    pub const F1: u8 = 0x70;
    pub const F2: u8 = 0x71;
    pub const F3: u8 = 0x72;
    pub const F4: u8 = 0x73;
    pub const F5: u8 = 0x74;
    pub const F6: u8 = 0x75;
    pub const F7: u8 = 0x76;
    pub const F8: u8 = 0x77;
    pub const F9: u8 = 0x78;
}

static KEYS: [AtomicBool; 256] = [const { AtomicBool::new(false) }; 256];
static CLOSE: AtomicBool = AtomicBool::new(false);
static MOUSE_X: AtomicI32 = AtomicI32::new(-1);
static MOUSE_Y: AtomicI32 = AtomicI32::new(-1);
static MOUSE_BTN: AtomicU32 = AtomicU32::new(0);
static WHEEL: AtomicI32 = AtomicI32::new(0);
static CHARS: Mutex<Vec<char>> = Mutex::new(Vec::new());

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            KEYS[wp & 0xFF].store(true, Ordering::Relaxed);
            if msg == WM_SYSKEYDOWN {
                return DefWindowProcW(hwnd, msg, wp, lp); // mantém Alt+F4
            }
            0
        }
        WM_KEYUP | WM_SYSKEYUP => {
            KEYS[wp & 0xFF].store(false, Ordering::Relaxed);
            0
        }
        WM_KILLFOCUS => {
            for k in KEYS.iter() {
                k.store(false, Ordering::Relaxed);
            }
            0
        }
        // Arrastar a janela trava o loop principal dentro do loop modal do Windows:
        // o watchdog não pode confundir isso com travamento.
        WM_ENTERSIZEMOVE => {
            crate::crash::WATCHDOG_PAUSED.store(true, Ordering::Relaxed);
            0
        }
        WM_EXITSIZEMOVE => {
            crate::crash::WATCHDOG_PAUSED.store(false, Ordering::Relaxed);
            0
        }
        WM_MOUSEMOVE => {
            MOUSE_X.store((lp & 0xFFFF) as i16 as i32, Ordering::Relaxed);
            MOUSE_Y.store(((lp >> 16) & 0xFFFF) as i16 as i32, Ordering::Relaxed);
            0
        }
        WM_LBUTTONDOWN => {
            MOUSE_BTN.fetch_or(1, Ordering::Relaxed);
            0
        }
        WM_LBUTTONUP => {
            MOUSE_BTN.fetch_and(!1, Ordering::Relaxed);
            0
        }
        WM_RBUTTONDOWN => {
            MOUSE_BTN.fetch_or(2, Ordering::Relaxed);
            0
        }
        WM_RBUTTONUP => {
            MOUSE_BTN.fetch_and(!2, Ordering::Relaxed);
            0
        }
        WM_MOUSEWHEEL => {
            WHEEL.fetch_add(((wp >> 16) & 0xFFFF) as i16 as i32, Ordering::Relaxed);
            0
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wp as u32) {
                if !c.is_control() {
                    CHARS.lock().unwrap_or_else(|e| e.into_inner()).push(c);
                }
            }
            0
        }
        WM_CLOSE | WM_DESTROY => {
            CLOSE.store(true, Ordering::Relaxed);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Mouse {
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub right: bool,
    pub left_pressed: bool,
    pub right_pressed: bool,
    /// Rolagem acumulada no frame (120 por "clique" da roda).
    pub wheel: i32,
}

impl Mouse {
    pub fn inside(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
        self.x >= x0 && self.x < x1 && self.y >= y0 && self.y < y1
    }
}

pub struct Keys {
    pub down: [bool; 256],
    pub pressed: [bool; 256],
}

impl Keys {
    pub fn down(&self, k: u8) -> bool {
        self.down[k as usize]
    }
    pub fn pressed(&self, k: u8) -> bool {
        self.pressed[k as usize]
    }
}

#[derive(Clone, Debug)]
pub struct Text {
    pub x: i32,
    pub y: i32,
    pub color: u32,
    pub big: bool,
    pub s: String,
}

impl Text {
    pub fn new(x: i32, y: i32, color: u32, s: impl Into<String>) -> Text {
        Text { x, y, color, big: false, s: s.into() }
    }
    pub fn big(x: i32, y: i32, color: u32, s: impl Into<String>) -> Text {
        Text { x, y, color, big: true, s: s.into() }
    }
}

/// Largura aproximada de um caractere da fonte pequena (Consolas 14 px).
pub const CHAR_W: i32 = 8;
pub const LINE_H: i32 = 15;

struct Surface {
    mem_dc: HDC,
    dib: HANDLE,
    bits: *mut u32,
    font: HANDLE,
    font_big: HANDLE,
    w: usize,
    h: usize,
}

impl Surface {
    unsafe fn new(ref_dc: HDC, w: usize, h: usize) -> Surface {
        let mem_dc = CreateCompatibleDC(ref_dc);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [0],
        };
        let mut bits: *mut c_void = null_mut();
        let dib = CreateDIBSection(mem_dc, &bmi, 0, &mut bits, null_mut(), 0);
        assert!(!dib.is_null() && !bits.is_null(), "CreateDIBSection falhou");
        SelectObject(mem_dc, dib);
        let face = wide("Consolas");
        // ANTIALIASED_QUALITY, FIXED_PITCH | FF_MODERN
        let font = CreateFontW(-14, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 4, 0x31, face.as_ptr());
        let font_big = CreateFontW(-30, 0, 0, 0, 800, 1, 0, 0, 1, 0, 0, 4, 0x31, face.as_ptr());
        SelectObject(mem_dc, font);
        SetBkMode(mem_dc, 1); // TRANSPARENT
        Surface { mem_dc, dib, bits: bits as *mut u32, font, font_big, w, h }
    }

    /// Copia os pixels para o DIB e desenha os textos com GDI por cima.
    unsafe fn compose(&self, px: &[u32], texts: &[Text]) {
        assert_eq!(px.len(), self.w * self.h);
        GdiFlush(); // GDI pode ter operações pendentes sobre o DIB
        std::ptr::copy_nonoverlapping(px.as_ptr(), self.bits, px.len());
        let mut big = false;
        for t in texts {
            if t.big != big {
                SelectObject(self.mem_dc, if t.big { self.font_big } else { self.font });
                big = t.big;
            }
            let s: Vec<u16> = t.s.encode_utf16().collect();
            let off = if t.big { 2 } else { 1 };
            SetTextColor(self.mem_dc, 0);
            TextOutW(self.mem_dc, t.x + off, t.y + off, s.as_ptr(), s.len() as i32);
            SetTextColor(self.mem_dc, colorref(t.color));
            TextOutW(self.mem_dc, t.x, t.y, s.as_ptr(), s.len() as i32);
        }
        if big {
            SelectObject(self.mem_dc, self.font);
        }
        GdiFlush();
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.mem_dc);
            DeleteObject(self.dib);
            DeleteObject(self.font);
            DeleteObject(self.font_big);
        }
    }
}

/// Desenha textos GDI sobre um buffer de pixels sem janela (capturas headless).
pub fn render_texts_offscreen(w: usize, h: usize, px: &mut [u32], texts: &[Text]) {
    unsafe {
        let s = Surface::new(null_mut(), w, h);
        s.compose(px, texts);
        std::ptr::copy_nonoverlapping(s.bits, px.as_mut_ptr(), px.len());
    }
}

pub struct Window {
    hwnd: HWND,
    hdc: HDC,
    surface: Surface,
    pub w: usize,
    pub h: usize,
    pub keys: Keys,
    pub mouse: Mouse,
    /// Caracteres digitados neste frame (para campos de texto).
    pub chars: Vec<char>,
    pub quit: bool,
}

impl Window {
    pub fn open(title: &str, w: usize, h: usize) -> Window {
        unsafe {
            timeBeginPeriod(1);
            let inst = GetModuleHandleW(null());
            let class = wide("ForjaWindow");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0x20 | 0x2 | 0x1, // CS_OWNDC | CS_HREDRAW | CS_VREDRAW
                lpfnWndProc: wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: inst,
                hIcon: null_mut(),
                hCursor: LoadCursorW(null_mut(), 32512 as *const u16), // IDC_ARROW
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
                hIconSm: null_mut(),
            };
            RegisterClassExW(&wc);
            // WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
            let style = 0x00C0_0000 | 0x0008_0000 | 0x0002_0000;
            let mut r = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
            AdjustWindowRect(&mut r, style, 0);
            let title_w = wide(title);
            let hwnd = CreateWindowExW(
                0, class.as_ptr(), title_w.as_ptr(), style | 0x1000_0000, // WS_VISIBLE
                0x8000_0000u32 as i32, 0x8000_0000u32 as i32, r.right - r.left, r.bottom - r.top,
                null_mut(), null_mut(), inst, null_mut(),
            );
            assert!(!hwnd.is_null(), "CreateWindowExW falhou");
            let hdc = GetDC(hwnd);
            let surface = Surface::new(hdc, w, h);
            Window {
                hwnd,
                hdc,
                surface,
                w,
                h,
                keys: Keys { down: [false; 256], pressed: [false; 256] },
                mouse: Mouse::default(),
                chars: Vec::new(),
                quit: false,
            }
        }
    }

    pub fn pump(&mut self) {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, 1) != 0 {
                if msg.message == WM_QUIT {
                    CLOSE.store(true, Ordering::Relaxed);
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        for i in 0..256 {
            let d = KEYS[i].load(Ordering::Relaxed);
            self.keys.pressed[i] = d && !self.keys.down[i];
            self.keys.down[i] = d;
        }
        let btn = MOUSE_BTN.load(Ordering::Relaxed);
        let (l, r) = (btn & 1 != 0, btn & 2 != 0);
        self.mouse = Mouse {
            x: MOUSE_X.load(Ordering::Relaxed),
            y: MOUSE_Y.load(Ordering::Relaxed),
            left_pressed: l && !self.mouse.left,
            right_pressed: r && !self.mouse.right,
            left: l,
            right: r,
            wheel: WHEEL.swap(0, Ordering::Relaxed),
        };
        self.chars = std::mem::take(&mut *CHARS.lock().unwrap_or_else(|e| e.into_inner()));
        self.quit = CLOSE.load(Ordering::Relaxed);
    }

    /// Desiste de fechar (ex.: usuário cancelou o "salvar alterações?").
    pub fn cancel_quit(&mut self) {
        CLOSE.store(false, Ordering::Relaxed);
        self.quit = false;
    }

    pub fn set_title(&self, s: &str) {
        let w = wide(s);
        unsafe {
            SetWindowTextW(self.hwnd, w.as_ptr());
        }
    }

    /// Copia o framebuffer para o DIB, desenha os textos com GDI e apresenta.
    pub fn present(&mut self, px: &[u32], texts: &[Text]) {
        unsafe {
            self.surface.compose(px, texts);
            BitBlt(self.hdc, 0, 0, self.w as i32, self.h as i32, self.surface.mem_dc, 0, 0, 0x00CC_0020); // SRCCOPY
        }
    }
}

fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

pub fn local_time() -> SYSTEMTIME {
    let mut st = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut st) };
    st
}

/// Pergunta sim/não/cancelar. Retorna Some(true) = sim, Some(false) = não, None = cancelar.
pub fn ask_yes_no_cancel(title: &str, text: &str) -> Option<bool> {
    let t = wide(title);
    let m = wide(text);
    match unsafe { MessageBoxW(null_mut(), m.as_ptr(), t.as_ptr(), 0x3 | 0x20) } {
        6 => Some(true),
        7 => Some(false),
        _ => None,
    }
}

pub fn message_box(title: &str, text: &str) {
    let t = wide(title);
    let m = wide(text);
    unsafe {
        MessageBoxW(null_mut(), m.as_ptr(), t.as_ptr(), 0x10); // MB_ICONERROR
    }
}
