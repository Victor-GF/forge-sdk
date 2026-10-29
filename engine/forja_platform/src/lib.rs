//! Plataforma: janela, input e relógio via Win32 direto.
//!
//! O jogo é dono do loop: chama `platform_poll` uma vez por frame e decide o resto.
//! Nenhum estado global: a janela não guarda ponteiro para `Platform`; o input é lido
//! direto da fila de mensagens em `platform_poll`, e tamanho/foco são consultados lá.
//!
//! Limitação conhecida: enquanto o usuário arrasta ou redimensiona a janela, o Windows
//! roda um loop modal próprio e o frame do jogo para até ele soltar.

pub mod input;

use core::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::Sleep;
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

use input::{
    INPUT_VAZIO, Input, MOUSE_DIREITO, MOUSE_ESQUERDO, MOUSE_MEIO, input_frame_inicio,
    input_mouse_evento, input_mouse_mover, input_roda_evento, input_soltar_tudo,
    input_tecla_evento,
};

pub struct JanelaDesc<'a> {
    pub titulo: &'a str,
    /// Tamanho da área cliente, em pixels físicos.
    pub largura: u32,
    pub altura: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlatformErro {
    RegistrarClasse,
    CriarJanela,
}

pub struct Platform {
    pub hwnd: HWND,
    pub hinstance: HINSTANCE,
    /// Nome da classe da janela em UTF-16 (terminado em 0), para `platform_shutdown`.
    pub classe: Vec<u16>,

    /// Fica `false` quando o usuário fecha a janela.
    pub rodando: bool,
    pub focado: bool,
    /// Área cliente atual; 0x0 quando minimizada.
    pub largura: u32,
    pub altura: u32,
    /// `true` no frame em que o tamanho mudou.
    pub redimensionou: bool,

    pub input: Input,

    pub ticks_por_segundo: u64,
    /// Tick em que o frame atual começou (`platform_poll`).
    pub frame_inicio: u64,
    pub frame_indice: u64,
    /// Segundos desde o `platform_poll` anterior. Sem limite: o jogo decide se limita.
    pub dt: f32,
}

// ---------------------------------------------------------------- relógio

/// Contador de alta resolução (QueryPerformanceCounter).
pub fn platform_ticks() -> u64 {
    let mut v: i64 = 0;
    // SAFETY: ponteiro para variável local válida.
    unsafe { QueryPerformanceCounter(&mut v) };
    u64::try_from(v).unwrap_or(0)
}

pub fn ticks_para_segundos(ticks: u64, ticks_por_segundo: u64) -> f64 {
    if ticks_por_segundo == 0 {
        return 0.0;
    }
    ticks as f64 / ticks_por_segundo as f64
}

/// Dorme até `alvo_segundos` depois do início do frame. Usa `Sleep` enquanto falta
/// bastante e gira (spin) nos últimos ~2 ms, porque `Sleep` não é preciso.
pub fn platform_esperar_frame(p: &Platform, alvo_segundos: f64) {
    let alvo_ticks = (alvo_segundos * p.ticks_por_segundo as f64) as u64;
    let fim = p.frame_inicio.saturating_add(alvo_ticks);
    loop {
        let agora = platform_ticks();
        if agora >= fim {
            return;
        }
        let resto_ms = ticks_para_segundos(fim.saturating_sub(agora), p.ticks_por_segundo) * 1000.0;
        if resto_ms > 2.0 {
            // SAFETY: chamada simples sem ponteiros.
            unsafe { Sleep((resto_ms - 1.5) as u32) };
        } else {
            core::hint::spin_loop();
        }
    }
}

// ---------------------------------------------------------------- janela

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

pub fn platform_init(desc: &JanelaDesc) -> Result<Platform, PlatformErro> {
    let classe = utf16("forja_janela");
    let titulo = utf16(desc.titulo);
    let largura = i32::try_from(desc.largura).unwrap_or(i32::MAX);
    let altura = i32::try_from(desc.altura).unwrap_or(i32::MAX);

    // SAFETY: chamadas Win32 com ponteiros para dados locais vivos durante a chamada;
    // `classe` e `titulo` terminam em 0.
    let (hwnd, hinstance) = unsafe {
        // Pixels físicos em telas com escala (pode falhar se já foi definido; tudo bem).
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Resolução de 1 ms para `Sleep` em `platform_esperar_frame`.
        timeBeginPeriod(1);

        let hinstance = GetModuleHandleW(null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(janela_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(BLACK_BRUSH),
            lpszMenuName: null(),
            lpszClassName: classe.as_ptr(),
            hIconSm: null_mut(),
        };
        if RegisterClassExW(&wc) == 0 {
            timeEndPeriod(1);
            return Err(PlatformErro::RegistrarClasse);
        }

        let estilo = WS_OVERLAPPEDWINDOW;
        let mut r = RECT { left: 0, top: 0, right: largura, bottom: altura };
        AdjustWindowRectEx(&mut r, estilo, 0, 0);

        let hwnd = CreateWindowExW(
            0,
            classe.as_ptr(),
            titulo.as_ptr(),
            estilo,
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
            UnregisterClassW(classe.as_ptr(), hinstance);
            timeEndPeriod(1);
            return Err(PlatformErro::CriarJanela);
        }
        ShowWindow(hwnd, SW_SHOW);
        (hwnd, hinstance)
    };

    let mut freq: i64 = 0;
    // SAFETY: ponteiro para variável local válida.
    unsafe { QueryPerformanceFrequency(&mut freq) };

    Ok(Platform {
        hwnd,
        hinstance,
        classe,
        rodando: true,
        focado: true,
        largura: desc.largura,
        altura: desc.altura,
        redimensionou: false,
        input: INPUT_VAZIO,
        ticks_por_segundo: u64::try_from(freq).unwrap_or(1).max(1),
        frame_inicio: platform_ticks(),
        frame_indice: 0,
        dt: 0.0,
    })
}

pub fn platform_shutdown(p: Platform) {
    // SAFETY: `hwnd` e `classe` vieram de `platform_init` e ainda são válidos.
    unsafe {
        DestroyWindow(p.hwnd);
        UnregisterClassW(p.classe.as_ptr(), p.hinstance);
        timeEndPeriod(1);
    }
}

pub fn platform_titulo(p: &Platform, titulo: &str) {
    let t = utf16(titulo);
    // SAFETY: `t` termina em 0 e vive durante a chamada.
    unsafe { SetWindowTextW(p.hwnd, t.as_ptr()) };
}

/// Processa as mensagens do SO e atualiza input, tamanho, foco e tempo.
/// Devolve `false` quando a janela foi fechada.
pub fn platform_poll(p: &mut Platform) -> bool {
    input_frame_inicio(&mut p.input);

    // SAFETY: MSG é uma struct C só com inteiros e ponteiros; zero é um valor válido.
    let mut msg: MSG = unsafe { core::mem::zeroed() };
    // SAFETY: `msg` é local e válido; hwnd nulo = todas as mensagens da thread.
    while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
        match msg.message {
            WM_QUIT => p.rodando = false,
            WM_KEYDOWN | WM_SYSKEYDOWN => input_tecla_evento(&mut p.input, tecla_de_wparam(msg.wParam), true),
            WM_KEYUP | WM_SYSKEYUP => input_tecla_evento(&mut p.input, tecla_de_wparam(msg.wParam), false),
            WM_MOUSEMOVE => input_mouse_mover(&mut p.input, lparam_x(msg.lParam), lparam_y(msg.lParam)),
            WM_LBUTTONDOWN => input_mouse_evento(&mut p.input, MOUSE_ESQUERDO, true),
            WM_LBUTTONUP => input_mouse_evento(&mut p.input, MOUSE_ESQUERDO, false),
            WM_RBUTTONDOWN => input_mouse_evento(&mut p.input, MOUSE_DIREITO, true),
            WM_RBUTTONUP => input_mouse_evento(&mut p.input, MOUSE_DIREITO, false),
            WM_MBUTTONDOWN => input_mouse_evento(&mut p.input, MOUSE_MEIO, true),
            WM_MBUTTONUP => input_mouse_evento(&mut p.input, MOUSE_MEIO, false),
            WM_MOUSEWHEEL => input_roda_evento(&mut p.input, roda_de_wparam(msg.wParam)),
            _ => {}
        }
        // Sem TranslateMessage: o MVP não tem entrada de texto (WM_CHAR).
        // SAFETY: `msg` veio de PeekMessageW.
        unsafe { DispatchMessageW(&msg) };
    }

    // SAFETY: chamada sem ponteiros.
    let focado = unsafe { GetForegroundWindow() } == p.hwnd;
    if p.focado && !focado {
        input_soltar_tudo(&mut p.input);
    }
    p.focado = focado;

    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: `r` é local e válido.
    unsafe { GetClientRect(p.hwnd, &mut r) };
    let largura = u32::try_from(r.right.saturating_sub(r.left)).unwrap_or(0);
    let altura = u32::try_from(r.bottom.saturating_sub(r.top)).unwrap_or(0);
    p.redimensionou = largura != p.largura || altura != p.altura;
    p.largura = largura;
    p.altura = altura;

    let agora = platform_ticks();
    p.dt = ticks_para_segundos(agora.saturating_sub(p.frame_inicio), p.ticks_por_segundo) as f32;
    p.frame_inicio = agora;
    p.frame_indice = p.frame_indice.wrapping_add(1);

    p.rodando
}

/// Sem estado: fechar vira WM_QUIT na fila (lido em `platform_poll`), e o menu do Alt/F10
/// é bloqueado para não pausar o jogo num loop modal.
unsafe extern "system" fn janela_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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

// ---------------------------------------------------------------- decodificação

fn tecla_de_wparam(w: WPARAM) -> u8 {
    (w & 0xFF) as u8
}

fn lparam_x(l: LPARAM) -> i32 {
    ((l as usize) & 0xFFFF) as u16 as i16 as i32
}

fn lparam_y(l: LPARAM) -> i32 {
    ((l as usize).wrapping_shr(16) & 0xFFFF) as u16 as i16 as i32
}

fn roda_de_wparam(w: WPARAM) -> i32 {
    (w.wrapping_shr(16) & 0xFFFF) as u16 as i16 as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordenadas_negativas_do_mouse() {
        // x = -5, y = 300 (mouse fora da janela à esquerda, com captura)
        let l = ((300usize << 16) | 0xFFFB) as LPARAM;
        assert_eq!(lparam_x(l), -5);
        assert_eq!(lparam_y(l), 300);
    }

    #[test]
    fn roda_para_tras_e_negativa() {
        let w = ((-120i16 as u16 as usize) << 16) as WPARAM;
        assert_eq!(roda_de_wparam(w), -120);
    }

    #[test]
    fn ticks_sem_frequencia_nao_divide_por_zero() {
        assert_eq!(ticks_para_segundos(100, 0), 0.0);
        assert_eq!(ticks_para_segundos(500, 1000), 0.5);
    }
}
