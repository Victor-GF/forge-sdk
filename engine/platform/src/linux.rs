//! Backend de janela do Linux: X11 (Xlib).
//!
//! A `libX11.so.6` é carregada em tempo de execução (`forja_os::library`), como o SDL faz: não
//! precisa do pacote de desenvolvimento para compilar, e o mesmo binário roda em máquina sem
//! X11 (aí `init` devolve `DisplayLibraryMissing` sem a biblioteca ou `DisplayUnavailable` sem
//! servidor, e o jogo pode usar o modo headless). Em sessão Wayland (padrão do Ubuntu), a janela
//! roda pelo XWayland; no WSL, o WSLg faz o mesmo (`DISPLAY=:0`) e a janela aparece no Windows.
//!
//! As structs de evento abaixo reproduzem o layout de `Xlib.h` em 64 bits; os testes conferem
//! os tamanhos.
//!
//! Limitações atuais: erros assíncronos do X usam o handler padrão da Xlib (imprime e encerra);
//! sem entrada de texto (só teclas).

use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use core::ptr::{null, null_mut};

use forja_os::library::Library;

use crate::input::{Input, Key, MouseButton};
use crate::{PlatformError, TITLE_MAX, WindowDesc, WindowState};

const X11_LIBRARY: &str = "libX11.so.6";

// ---------------------------------------------------------------- tipos da Xlib

#[repr(C)]
struct Display {
    _private: [u8; 0],
}

type XWindow = c_ulong;
type Atom = c_ulong;
type KeySym = c_ulong;
type XBool = c_int;
type XTime = c_ulong;

#[repr(C)]
#[derive(Clone, Copy)]
struct XKeyEvent {
    kind: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    window: XWindow,
    root: XWindow,
    subwindow: XWindow,
    time: XTime,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    keycode: c_uint,
    same_screen: XBool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct XButtonEvent {
    kind: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    window: XWindow,
    root: XWindow,
    subwindow: XWindow,
    time: XTime,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    button: c_uint,
    same_screen: XBool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct XMotionEvent {
    kind: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    window: XWindow,
    root: XWindow,
    subwindow: XWindow,
    time: XTime,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    is_hint: c_char,
    same_screen: XBool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct XConfigureEvent {
    kind: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    event: XWindow,
    window: XWindow,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: XWindow,
    override_redirect: XBool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct XClientMessageEvent {
    kind: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    window: XWindow,
    message_type: Atom,
    format: c_int,
    data: [c_long; 5],
}

/// `XEvent` da Xlib: união de todos os eventos, com 24 `long`s no total.
#[repr(C)]
union XEvent {
    kind: c_int,
    key: XKeyEvent,
    button: XButtonEvent,
    motion: XMotionEvent,
    configure: XConfigureEvent,
    client: XClientMessageEvent,
    pad: [c_long; 24],
}

const KEY_PRESS: c_int = 2;
const KEY_RELEASE: c_int = 3;
const BUTTON_PRESS: c_int = 4;
const BUTTON_RELEASE: c_int = 5;
const MOTION_NOTIFY: c_int = 6;
const FOCUS_IN: c_int = 9;
const FOCUS_OUT: c_int = 10;
const UNMAP_NOTIFY: c_int = 18;
const MAP_NOTIFY: c_int = 19;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;

const KEY_PRESS_MASK: c_long = 1 << 0;
const KEY_RELEASE_MASK: c_long = 1 << 1;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const FOCUS_CHANGE_MASK: c_long = 1 << 21;
const EVENT_MASK: c_long = KEY_PRESS_MASK
    | KEY_RELEASE_MASK
    | BUTTON_PRESS_MASK
    | BUTTON_RELEASE_MASK
    | POINTER_MOTION_MASK
    | STRUCTURE_NOTIFY_MASK
    | FOCUS_CHANGE_MASK;

const PROP_MODE_REPLACE: c_int = 0;
/// Um "clique" da roda, no mesmo valor do Windows (`WHEEL_DELTA`).
const WHEEL_STEP: i32 = 120;

// ---------------------------------------------------------------- funções da Xlib

type XOpenDisplayFn = unsafe extern "C" fn(*const c_char) -> *mut Display;
type XCloseDisplayFn = unsafe extern "C" fn(*mut Display) -> c_int;
type XDefaultScreenFn = unsafe extern "C" fn(*mut Display) -> c_int;
type XRootWindowFn = unsafe extern "C" fn(*mut Display, c_int) -> XWindow;
type XBlackPixelFn = unsafe extern "C" fn(*mut Display, c_int) -> c_ulong;
type XCreateSimpleWindowFn = unsafe extern "C" fn(
    *mut Display,
    XWindow,
    c_int,
    c_int,
    c_uint,
    c_uint,
    c_uint,
    c_ulong,
    c_ulong,
) -> XWindow;
type XSelectInputFn = unsafe extern "C" fn(*mut Display, XWindow, c_long) -> c_int;
type XInternAtomFn = unsafe extern "C" fn(*mut Display, *const c_char, XBool) -> Atom;
type XSetWMProtocolsFn = unsafe extern "C" fn(*mut Display, XWindow, *mut Atom, c_int) -> c_int;
type XStoreNameFn = unsafe extern "C" fn(*mut Display, XWindow, *const c_char) -> c_int;
type XChangePropertyFn =
    unsafe extern "C" fn(*mut Display, XWindow, Atom, Atom, c_int, c_int, *const u8, c_int) -> c_int;
type XWindowFn = unsafe extern "C" fn(*mut Display, XWindow) -> c_int;
type XDisplayFn = unsafe extern "C" fn(*mut Display) -> c_int;
type XNextEventFn = unsafe extern "C" fn(*mut Display, *mut XEvent) -> c_int;
type XLookupKeysymFn = unsafe extern "C" fn(*mut XKeyEvent, c_int) -> KeySym;
type XkbSetDetectableAutoRepeatFn = unsafe extern "C" fn(*mut Display, XBool, *mut XBool) -> XBool;

/// As funções da Xlib que usamos, resolvidas da biblioteca carregada.
struct Xlib {
    open_display: XOpenDisplayFn,
    close_display: XCloseDisplayFn,
    default_screen: XDefaultScreenFn,
    root_window: XRootWindowFn,
    black_pixel: XBlackPixelFn,
    create_simple_window: XCreateSimpleWindowFn,
    select_input: XSelectInputFn,
    intern_atom: XInternAtomFn,
    set_wm_protocols: XSetWMProtocolsFn,
    store_name: XStoreNameFn,
    change_property: XChangePropertyFn,
    map_window: XWindowFn,
    destroy_window: XWindowFn,
    flush: XDisplayFn,
    pending: XDisplayFn,
    next_event: XNextEventFn,
    lookup_keysym: XLookupKeysymFn,
    set_detectable_auto_repeat: XkbSetDetectableAutoRepeatFn,
}

fn symbol(lib: &Library, name: &str) -> Result<*mut c_void, PlatformError> {
    lib.symbol(name).ok_or(PlatformError::DisplayLibraryMissing)
}

/// Resolve todas as funções. Cada `transmute` converte o endereço exportado para a assinatura
/// de `Xlib.h`.
fn load_xlib(lib: &Library) -> Result<Xlib, PlatformError> {
    // SAFETY: cada símbolo é a função da Xlib de mesmo nome, com a assinatura declarada acima.
    unsafe {
        Ok(Xlib {
            open_display: core::mem::transmute::<*mut c_void, XOpenDisplayFn>(symbol(lib, "XOpenDisplay")?),
            close_display: core::mem::transmute::<*mut c_void, XCloseDisplayFn>(symbol(lib, "XCloseDisplay")?),
            default_screen: core::mem::transmute::<*mut c_void, XDefaultScreenFn>(symbol(lib, "XDefaultScreen")?),
            root_window: core::mem::transmute::<*mut c_void, XRootWindowFn>(symbol(lib, "XRootWindow")?),
            black_pixel: core::mem::transmute::<*mut c_void, XBlackPixelFn>(symbol(lib, "XBlackPixel")?),
            create_simple_window: core::mem::transmute::<*mut c_void, XCreateSimpleWindowFn>(symbol(
                lib,
                "XCreateSimpleWindow",
            )?),
            select_input: core::mem::transmute::<*mut c_void, XSelectInputFn>(symbol(lib, "XSelectInput")?),
            intern_atom: core::mem::transmute::<*mut c_void, XInternAtomFn>(symbol(lib, "XInternAtom")?),
            set_wm_protocols: core::mem::transmute::<*mut c_void, XSetWMProtocolsFn>(symbol(lib, "XSetWMProtocols")?),
            store_name: core::mem::transmute::<*mut c_void, XStoreNameFn>(symbol(lib, "XStoreName")?),
            change_property: core::mem::transmute::<*mut c_void, XChangePropertyFn>(symbol(lib, "XChangeProperty")?),
            map_window: core::mem::transmute::<*mut c_void, XWindowFn>(symbol(lib, "XMapWindow")?),
            destroy_window: core::mem::transmute::<*mut c_void, XWindowFn>(symbol(lib, "XDestroyWindow")?),
            flush: core::mem::transmute::<*mut c_void, XDisplayFn>(symbol(lib, "XFlush")?),
            pending: core::mem::transmute::<*mut c_void, XDisplayFn>(symbol(lib, "XPending")?),
            next_event: core::mem::transmute::<*mut c_void, XNextEventFn>(symbol(lib, "XNextEvent")?),
            lookup_keysym: core::mem::transmute::<*mut c_void, XLookupKeysymFn>(symbol(lib, "XLookupKeysym")?),
            set_detectable_auto_repeat: core::mem::transmute::<*mut c_void, XkbSetDetectableAutoRepeatFn>(
                symbol(lib, "XkbSetDetectableAutoRepeat")?,
            ),
        })
    }
}

// ---------------------------------------------------------------- janela

pub(crate) struct Window {
    x: Xlib,
    library: Library,
    display: *mut Display,
    window: XWindow,
    wm_delete_window: Atom,
    net_wm_name: Atom,
    utf8_string: Atom,
    focused: bool,
    /// `false` enquanto minimizada: o gerenciador de janelas tira a janela da tela (ICCCM) sem
    /// mudar o tamanho, então o 0x0 do contrato sai daqui.
    mapped: bool,
    /// Último tamanho do ConfigureNotify, mantido enquanto minimizada.
    width: u32,
    height: u32,
}

pub(crate) fn window_create(desc: &WindowDesc) -> Result<Window, PlatformError> {
    // Mesmo limite do Windows: o título precisa caber em TITLE_MAX unidades UTF-16 com o 0.
    if desc.title.encode_utf16().count() >= TITLE_MAX {
        return Err(PlatformError::TitleTooLong);
    }
    let Some(library) = Library::open(X11_LIBRARY) else {
        return Err(PlatformError::DisplayLibraryMissing);
    };
    let x = match load_xlib(&library) {
        Ok(x) => x,
        Err(e) => {
            library.close();
            return Err(e);
        }
    };
    // SAFETY: nulo = usar a variável DISPLAY; o resultado é checado.
    let display = unsafe { (x.open_display)(null()) };
    if display.is_null() {
        library.close();
        return Err(PlatformError::DisplayUnavailable);
    }
    let width = c_uint::try_from(desc.width).unwrap_or(c_uint::MAX).max(1);
    let height = c_uint::try_from(desc.height).unwrap_or(c_uint::MAX).max(1);

    // SAFETY: `display` é válido; strings são literais C; `wm_delete_window` é local.
    unsafe {
        let screen = (x.default_screen)(display);
        let root = (x.root_window)(display, screen);
        let black = (x.black_pixel)(display, screen);
        let window = (x.create_simple_window)(display, root, 0, 0, width, height, 0, black, black);
        if window == 0 {
            (x.close_display)(display);
            library.close();
            return Err(PlatformError::CreateWindow);
        }
        (x.select_input)(display, window, EVENT_MASK);

        // O botão de fechar vira uma ClientMessage com WM_DELETE_WINDOW em vez de matar a conexão.
        let mut wm_delete_window = (x.intern_atom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
        (x.set_wm_protocols)(display, window, &mut wm_delete_window, 1);
        let net_wm_name = (x.intern_atom)(display, c"_NET_WM_NAME".as_ptr(), 0);
        let utf8_string = (x.intern_atom)(display, c"UTF8_STRING".as_ptr(), 0);

        // Sem isso, segurar uma tecla gera pares soltou/apertou falsos.
        (x.set_detectable_auto_repeat)(display, 1, null_mut());
        (x.map_window)(display, window);

        let w = Window {
            x,
            library,
            display,
            window,
            wm_delete_window,
            net_wm_name,
            utf8_string,
            focused: true,
            mapped: true,
            width: desc.width,
            height: desc.height,
        };
        window_set_title(&w, desc.title);
        (w.x.flush)(display);
        Ok(w)
    }
}

pub(crate) fn window_destroy(w: Window) {
    // SAFETY: `display` e `window` vieram de `window_create` e ainda são válidos.
    unsafe {
        (w.x.destroy_window)(w.display, w.window);
        (w.x.close_display)(w.display);
    }
    w.library.close();
}

/// Títulos que não cabem em `TITLE_MAX` (ou com byte zero) são ignorados.
pub(crate) fn window_set_title(w: &Window, title: &str) {
    if title.encode_utf16().count() >= TITLE_MAX || title.contains('\0') {
        return;
    }
    let mut buf = [0u8; TITLE_MAX * 4 + 1];
    let len = title.len();
    let (Some(dst), Ok(len_c)) = (buf.get_mut(..len), c_int::try_from(len)) else {
        return;
    };
    dst.copy_from_slice(title.as_bytes());
    // SAFETY: `buf` termina em 0 (o resto do buffer é zero); `display`/`window` são válidos.
    // XStoreName é o título clássico (Latin-1); _NET_WM_NAME em UTF-8 é o que os gerenciadores
    // modernos mostram.
    unsafe {
        (w.x.store_name)(w.display, w.window, buf.as_ptr().cast::<c_char>());
        (w.x.change_property)(w.display, w.window, w.net_wm_name, w.utf8_string, 8, PROP_MODE_REPLACE, buf.as_ptr(), len_c);
        (w.x.flush)(w.display);
    }
}

pub(crate) fn window_poll(w: &mut Window, input: &mut Input) -> WindowState {
    let mut closed = false;
    // SAFETY: `display` é válido; `ev` é preenchido por XNextEvent antes de ser lido, e cada campo
    // da união só é lido quando `kind` diz que é aquele evento.
    unsafe {
        while (w.x.pending)(w.display) > 0 {
            let mut ev: XEvent = core::mem::zeroed();
            (w.x.next_event)(w.display, &mut ev);
            match ev.kind {
                KEY_PRESS | KEY_RELEASE => {
                    let mut k = ev.key;
                    if let Some(key) = key_from_keysym((w.x.lookup_keysym)(&mut k, 0)) {
                        input.key_event(key, ev.kind == KEY_PRESS);
                    }
                }
                BUTTON_PRESS | BUTTON_RELEASE => {
                    let b = ev.button;
                    let down = ev.kind == BUTTON_PRESS;
                    input.mouse_move(b.x, b.y);
                    match b.button {
                        1 => input.mouse_button_event(MouseButton::Left, down),
                        2 => input.mouse_button_event(MouseButton::Middle, down),
                        3 => input.mouse_button_event(MouseButton::Right, down),
                        // Roda: X11 manda como botões 4 (para frente) e 5 (para trás).
                        4 if down => input.wheel_event(WHEEL_STEP),
                        5 if down => input.wheel_event(-WHEEL_STEP),
                        _ => {}
                    }
                }
                MOTION_NOTIFY => input.mouse_move(ev.motion.x, ev.motion.y),
                FOCUS_IN => w.focused = true,
                FOCUS_OUT => w.focused = false,
                MAP_NOTIFY => w.mapped = true,
                UNMAP_NOTIFY => w.mapped = false,
                CONFIGURE_NOTIFY => {
                    w.width = u32::try_from(ev.configure.width).unwrap_or(0);
                    w.height = u32::try_from(ev.configure.height).unwrap_or(0);
                }
                CLIENT_MESSAGE => {
                    let atom = ev.client.data.first().copied().unwrap_or(0) as Atom;
                    if atom == w.wm_delete_window {
                        closed = true;
                    }
                }
                _ => {}
            }
        }
    }
    let (width, height) = if w.mapped { (w.width, w.height) } else { (0, 0) };
    WindowState { closed, focused: w.focused, width, height }
}

/// Converte o keysym do X11 para `Key` (códigos virtuais do Windows, ver `input.rs`).
/// `None` para teclas que ainda não mapeamos.
fn key_from_keysym(sym: KeySym) -> Option<Key> {
    let key = match sym {
        // a-z: o índice 0 do XLookupKeysym dá a minúscula; Key usa a maiúscula.
        0x61..=0x7A => Key(u8::try_from(sym).ok()?.wrapping_sub(0x20)),
        // 0-9 e espaço: mesmo código ASCII.
        0x30..=0x39 | 0x20 => Key(u8::try_from(sym).ok()?),
        0xFF08 => Key::BACKSPACE,
        0xFF09 => Key::TAB,
        0xFF0D => Key::ENTER,
        0xFF1B => Key::ESCAPE,
        0xFF51 => Key::LEFT,
        0xFF52 => Key::UP,
        0xFF53 => Key::RIGHT,
        0xFF54 => Key::DOWN,
        0xFFE1 | 0xFFE2 => Key::SHIFT,
        0xFFE3 | 0xFFE4 => Key::CONTROL,
        0xFFE9 | 0xFFEA => Key::ALT,
        // F1-F12 são consecutivos nos dois sistemas.
        0xFFBE..=0xFFC9 => Key(Key::F1.0.wrapping_add(u8::try_from(sym.wrapping_sub(0xFFBE)).ok()?)),
        _ => return None,
    };
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xlib_event_layouts_match_64_bit_headers() {
        // Tamanhos de Xlib.h em x86_64: se mudarem, a leitura dos eventos sai errada.
        assert_eq!(size_of::<XEvent>(), 192);
        assert_eq!(size_of::<XKeyEvent>(), 96);
        assert_eq!(size_of::<XButtonEvent>(), 96);
        assert_eq!(size_of::<XMotionEvent>(), 96);
        assert_eq!(size_of::<XConfigureEvent>(), 88);
        assert_eq!(size_of::<XClientMessageEvent>(), 96);
    }

    #[test]
    fn keysyms_map_to_the_same_keys_as_windows() {
        assert_eq!(key_from_keysym(0x77), Some(Key(b'W'))); // XK_w
        assert_eq!(key_from_keysym(0x73), Some(Key(b'S'))); // XK_s
        assert_eq!(key_from_keysym(0x35), Some(Key(b'5')));
        assert_eq!(key_from_keysym(0x20), Some(Key::SPACE));
        assert_eq!(key_from_keysym(0xFF1B), Some(Key::ESCAPE));
        assert_eq!(key_from_keysym(0xFF52), Some(Key::UP));
        assert_eq!(key_from_keysym(0xFFE2), Some(Key::SHIFT));
        assert_eq!(key_from_keysym(0xFFBE), Some(Key::F1));
        assert_eq!(key_from_keysym(0xFFC9), Some(Key::F12));
        assert_eq!(key_from_keysym(0xFFFF), None);
    }
}
