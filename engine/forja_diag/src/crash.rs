//! Crash report.
//!
//! Entradas:
//! - `crash::fatal(reason, details)`: erro fatal decidido pelo código (OOM de arena, alocador
//!   travado...). Grava o relatório e aborta.
//! - panic: o hook instalado por `crash::install` grava e aborta antes de a std alocar o
//!   payload do unwind.
//! - exceções do SO (access violation, stack overflow, instrução ilegal...): filtro de
//!   exceção não tratada.
//!
//! O `abort()` do Rust no Windows usa *fast fail*, que pula os filtros de exceção; por isso todo
//! caminho que aborta de propósito passa por `crash::fatal` antes.
//!
//! O relatório é montado num buffer estático (`REPORT_CAPACITY`) e gravado via Win32:
//! nada é alocado, então funciona sem memória livre e com o alocador global travado.
//! Pouca stack é usada, porque no stack overflow o handler roda com ~20 KB de reserva.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use forja_core::text::TextBuf;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile,
};
use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::SystemInformation::{
    GetLocalTime, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, Sleep, TerminateProcess,
};

use crate::log::write_breadcrumbs;
use crate::spin_lock::SpinLock;
use crate::stack;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrashConfig {
    pub app: &'static str,
    pub version: &'static str,
    /// Pasta dos relatórios (um nível, criada se não existir).
    pub dir: &'static str,
}

const DEFAULT_CONFIG: CrashConfig = CrashConfig { app: "?", version: "?", dir: "crash" };

/// Seção extra do relatório: o jogo e a engine registram funções que escrevem o próprio
/// estado. Rodam durante o crash: não podem alocar nem entrar em pânico.
#[derive(Clone, Copy)]
pub struct Section {
    pub name: &'static str,
    pub write: fn(&mut TextBuf),
}

pub const SECTIONS_MAX: usize = 16;
pub const REPORT_CAPACITY: usize = 64 * 1024;
const EXCEPTION_EXECUTE_HANDLER: i32 = 1;

#[derive(Clone, Copy)]
struct State {
    config: CrashConfig,
    sections: [Option<Section>; SECTIONS_MAX],
    section_count: usize,
}

struct Shared {
    lock: SpinLock,
    state: UnsafeCell<State>,
}

// SAFETY: `state` só é escrito com `lock` pego; no crash é lido mesmo sem ele (melhor esforço).
unsafe impl Sync for Shared {}

static SHARED: Shared = Shared {
    lock: SpinLock::new(),
    state: UnsafeCell::new(State {
        config: DEFAULT_CONFIG,
        sections: [None; SECTIONS_MAX],
        section_count: 0,
    }),
};

struct ReportBuf(UnsafeCell<[u8; REPORT_CAPACITY]>);

// SAFETY: só a thread que venceu `enter_crash` usa o buffer.
unsafe impl Sync for ReportBuf {}

static REPORT: ReportBuf = ReportBuf(UnsafeCell::new([0; REPORT_CAPACITY]));
static START_MS: AtomicU64 = AtomicU64::new(0);
static FRAME: AtomicU64 = AtomicU64::new(0);
/// Thread que está escrevendo o relatório (0 = nenhuma).
static CRASH_THREAD: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------- API

/// Instala o hook de panic e o filtro de exceções. Chame no início do `main`, antes de
/// travar o alocador global.
pub fn install(config: CrashConfig) {
    if SHARED.lock.try_lock(u32::MAX) {
        // SAFETY: lock pego.
        unsafe { (*SHARED.state.get()).config = config };
        SHARED.lock.unlock();
    }
    // SAFETY: chamadas sem ponteiros (o filtro é uma função estática).
    unsafe {
        START_MS.store(GetTickCount64(), Ordering::Relaxed);
        SetUnhandledExceptionFilter(Some(exception_filter));
    }
    // `panic_hook` é uma função (tamanho zero): o Box não aloca.
    std::panic::set_hook(Box::new(panic_hook));
}

/// Registra uma seção extra no relatório. `false` se já houver `SECTIONS_MAX`.
pub fn register_section(name: &'static str, write: fn(&mut TextBuf)) -> bool {
    if !SHARED.lock.try_lock(u32::MAX) {
        return false;
    }
    // SAFETY: lock pego.
    let s = unsafe { &mut *SHARED.state.get() };
    let ok = match s.sections.get_mut(s.section_count) {
        Some(slot) => {
            *slot = Some(Section { name, write });
            s.section_count = s.section_count.saturating_add(1);
            true
        }
        None => false,
    };
    SHARED.lock.unlock();
    ok
}

/// Frame atual, para o relatório. Barato: um store atômico.
pub fn set_frame(index: u64) {
    FRAME.store(index, Ordering::Relaxed);
}

/// Erro fatal: grava o relatório e aborta.
pub fn fatal(reason: &str, details: fmt::Arguments) -> ! {
    write_report(reason, details, None);
    std::process::abort()
}

// ---------------------------------------------------------------- entradas do SO e da std

fn panic_hook(info: &std::panic::PanicHookInfo) {
    let msg = info.payload_as_str().unwrap_or("(mensagem não textual)");
    match info.location() {
        Some(l) => write_report(
            "panic",
            format_args!("{msg}\n         em {}:{}:{}", l.file(), l.line(), l.column()),
            None,
        ),
        None => write_report("panic", format_args!("{msg}"), None),
    }
    std::process::abort()
}

unsafe extern "system" fn exception_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: o SO passa ponteiros válidos (checados contra nulo).
    let record = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
    let Some(r) = record else {
        write_report("exceção", format_args!("sem registro de exceção"), None);
        return EXCEPTION_EXECUTE_HANDLER;
    };
    let code = r.ExceptionCode as u32;
    let address = r.ExceptionAddress as usize;
    let p0 = r.ExceptionInformation.first().copied().unwrap_or(0);
    let p1 = r.ExceptionInformation.get(1).copied().unwrap_or(0);
    let name = exception_name(code);
    if code == 0xC000_0005 {
        let action = match p0 {
            0 => "ler",
            1 => "escrever",
            8 => "executar",
            _ => "acessar",
        };
        write_report(
            name,
            format_args!("código 0x{code:08X}: tentou {action} o endereço 0x{p1:016x}"),
            Some(address),
        );
    } else {
        write_report(name, format_args!("código 0x{code:08X}"), Some(address));
    }
    EXCEPTION_EXECUTE_HANDLER
}

fn exception_name(code: u32) -> &'static str {
    match code {
        0xC000_0005 => "access violation",
        0xC000_00FD => "stack overflow",
        0xC000_001D => "instrução ilegal",
        0xC000_0094 => "divisão inteira por zero",
        0xC000_0095 => "overflow inteiro",
        0xC000_008C => "índice de array fora dos limites",
        0xC000_0006 => "erro de página (I/O)",
        0x8000_0003 => "breakpoint",
        _ => "exceção do SO",
    }
}

// ---------------------------------------------------------------- relatório

/// Garante que só uma thread escreve o relatório. Crash dentro do crash (mesma thread)
/// encerra na hora; outras threads esperam o processo acabar.
fn enter_crash() -> bool {
    // SAFETY: chamada sem ponteiros.
    let me = unsafe { GetCurrentThreadId() };
    match CRASH_THREAD.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => true,
        Err(owner) if owner == me => {
            // SAFETY: encerra o próprio processo.
            unsafe { TerminateProcess(GetCurrentProcess(), 0xDEAD) };
            false
        }
        Err(_) => loop {
            // SAFETY: chamada sem ponteiros.
            unsafe { Sleep(1000) };
        },
    }
}

fn read_state() -> State {
    let locked = SHARED.lock.try_lock(1000);
    // SAFETY: com o lock, ou no crash (melhor esforço).
    let s = unsafe { *SHARED.state.get() };
    if locked {
        SHARED.lock.unlock();
    }
    s
}

fn write_report(reason: &str, details: fmt::Arguments, exception_address: Option<usize>) {
    if !enter_crash() {
        return;
    }
    let state = read_state();
    // SAFETY: só esta thread chega aqui (`enter_crash`).
    let buf: &mut [u8] = unsafe { &mut *REPORT.0.get() };
    let mut t = TextBuf::new(buf);

    // SAFETY: SYSTEMTIME só tem inteiros; zero é válido. Chamadas sem ponteiros externos.
    let (now, ms, thread) = unsafe {
        let mut st = core::mem::zeroed();
        GetLocalTime(&mut st);
        (st, GetTickCount64(), GetCurrentThreadId())
    };
    let uptime_s = ms.saturating_sub(START_MS.load(Ordering::Relaxed)) as f64 / 1000.0;

    let _ = writeln!(t, "=== Forja: relatório de crash ===");
    let _ = writeln!(t, "jogo:    {} {}", state.config.app, state.config.version);
    let _ = writeln!(
        t,
        "data:    {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    );
    let _ = writeln!(t, "rodando: {uptime_s:.3} s");
    let _ = writeln!(t, "frame:   {}", FRAME.load(Ordering::Relaxed));
    let _ = writeln!(t, "thread:  {thread}");
    let _ = writeln!(t, "motivo:  {reason}");
    let _ = writeln!(t, "detalhe: {details}");

    write_stack(&mut t, exception_address);
    write_system_memory(&mut t);
    for s in state.sections.iter().flatten() {
        let _ = writeln!(t, "\n--- {} ---", s.name);
        (s.write)(&mut t);
    }
    let _ = writeln!(t, "\n--- breadcrumbs (últimas linhas de log) ---");
    write_breadcrumbs(&mut t);
    if t.truncated() {
        let _ = t.write_str("\n[relatório cortado: buffer cheio]\n");
    }

    let mut path_buf = [0u8; 512];
    let mut path = TextBuf::new(&mut path_buf);
    let _ = write!(
        path,
        "{}/crash_{:04}-{:02}-{:02}_{:02}{:02}{:02}.txt",
        state.config.dir, now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    );
    create_dir(state.config.dir);
    let written = write_file(path.as_str(), t.as_bytes());

    let mut notice_buf = [0u8; 1024];
    let mut notice = TextBuf::new(&mut notice_buf);
    let _ = writeln!(notice, "[crash] {reason}: {details}");
    if written {
        let _ = writeln!(notice, "[crash] relatório em {}", path.as_str());
    } else {
        let _ = writeln!(notice, "[crash] falha ao gravar o relatório em {}", path.as_str());
    }
    let _ = std::io::Write::write_all(&mut std::io::stderr(), notice.as_bytes());
}

fn write_stack(t: &mut TextBuf, exception_address: Option<usize>) {
    let _ = writeln!(t, "\n--- pilha (módulo+deslocamento; simbolize com tools/symbolize.sh) ---");
    if let Some(address) = exception_address {
        let _ = write!(t, "exceção 0x{address:016x} ");
        stack::write_module_offset(t, address);
        let _ = t.write_char('\n');
    }
    let mut frames = [0usize; stack::FRAMES_MAX];
    let n = stack::capture(1, &mut frames);
    for (i, f) in frames.iter().take(n).enumerate() {
        let _ = write!(t, "#{i:02}     0x{f:016x} ");
        stack::write_module_offset(t, *f);
        let _ = t.write_char('\n');
    }
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn write_system_memory(t: &mut TextBuf) {
    let _ = writeln!(t, "\n--- memória do sistema ---");
    // SAFETY: structs C só com inteiros (zero é válido); tamanhos preenchidos antes da chamada.
    unsafe {
        let mut m: MEMORYSTATUSEX = core::mem::zeroed();
        m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 {
            let _ = writeln!(
                t,
                "RAM:      {:.0} MB livres de {:.0} MB (carga {}%)",
                mib(m.ullAvailPhys),
                mib(m.ullTotalPhys),
                m.dwMemoryLoad
            );
            let _ = writeln!(
                t,
                "commit:   {:.0} MB livres de {:.0} MB (limite do sistema)",
                mib(m.ullAvailPageFile),
                mib(m.ullTotalPageFile)
            );
        }
        let mut p: PROCESS_MEMORY_COUNTERS = core::mem::zeroed();
        p.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb) != 0 {
            let _ = writeln!(
                t,
                "processo: working set {:.1} MB (pico {:.1}), commit {:.1} MB (pico {:.1})",
                mib(p.WorkingSetSize as u64),
                mib(p.PeakWorkingSetSize as u64),
                mib(p.PagefileUsage as u64),
                mib(p.PeakPagefileUsage as u64)
            );
        }
    }
}

// ---------------------------------------------------------------- arquivo (Win32, sem alocar)

fn utf16_into<'b>(s: &str, buf: &'b mut [u16]) -> Option<&'b [u16]> {
    let mut n: usize = 0;
    for c in s.encode_utf16() {
        *buf.get_mut(n)? = c;
        n = n.checked_add(1)?;
    }
    *buf.get_mut(n)? = 0;
    buf.get(..=n)
}

fn create_dir(dir: &str) {
    let mut buf = [0u16; 260];
    if let Some(p) = utf16_into(dir, &mut buf) {
        // SAFETY: `p` termina em 0. Falha (pasta já existe) é ignorada.
        unsafe { CreateDirectoryW(p.as_ptr(), null()) };
    }
}

fn write_file(path: &str, data: &[u8]) -> bool {
    let mut buf = [0u16; 260];
    let Some(p) = utf16_into(path, &mut buf) else {
        return false;
    };
    // SAFETY: `p` termina em 0; sem atributos de segurança nem template.
    let h = unsafe {
        CreateFileW(p.as_ptr(), GENERIC_WRITE, 0, null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut rest = data;
    let mut ok = true;
    while !rest.is_empty() {
        let n = u32::try_from(rest.len()).unwrap_or(u32::MAX);
        let mut written: u32 = 0;
        // SAFETY: `rest` tem pelo menos `n` bytes; escrita síncrona.
        let r = unsafe { WriteFile(h, rest.as_ptr(), n, &mut written, null_mut()) };
        if r == 0 || written == 0 {
            ok = false;
            break;
        }
        rest = rest.get(written as usize..).unwrap_or(&[]);
    }
    // SAFETY: `h` é válido e fechado uma vez.
    unsafe { CloseHandle(h) };
    ok
}
