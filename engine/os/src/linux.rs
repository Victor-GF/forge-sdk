//! Backend Linux do `forja_os` (libc). Ver o contrato em `lib.rs`.
//!
//! Erro fatal: sinais (`SIGSEGV`, `SIGBUS`, `SIGILL`, `SIGFPE`) com `SA_ONSTACK` numa pilha
//! alternativa de 64 KB (vale para a thread que chamou `install`) e `SA_RESETHAND`: quando o
//! handler volta, a instrução repete e o processo morre pelo sinal (core dump e debugger
//! continuam funcionando). `SIGABRT` não é tratado: `abort()` sempre vem depois do relatório.
//!
//! Pilha: unwinder da libgcc (`_Unwind_Backtrace`, já linkado pela std), que atravessa o quadro
//! do sinal. Módulo: `dladdr`.

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt::Write;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, Ordering};

use forja_core::text::TextBuf;

use crate::fatal::{FatalEvent, FatalKind, MemoryAccess};
use crate::file::PATH_MAX;
use crate::library::NAME_MAX;
use crate::memory::PageInfo;
use crate::process::{ProcessMemory, SystemMemory};
use crate::time::UtcTime;

const NANOS_PER_SECOND: u64 = 1_000_000_000;

fn errno() -> c_int {
    // SAFETY: `__errno_location` devolve o errno da thread atual, sempre válido.
    unsafe { *libc::__errno_location() }
}

/// Copia `s` para `buf` com um 0 no fim (string C). `None` se não couber.
fn c_path<'b>(s: &str, buf: &'b mut [u8]) -> Option<&'b [u8]> {
    let n = s.len();
    buf.get_mut(..n)?.copy_from_slice(s.as_bytes());
    *buf.get_mut(n)? = 0;
    buf.get(..=n)
}

// ---------------------------------------------------------------- memória

pub(crate) fn page_info() -> PageInfo {
    // SAFETY: chamada sem ponteiros.
    let raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page = usize::try_from(raw).ok().filter(|p| p.is_power_of_two()).unwrap_or(4096);
    PageInfo { page_size: page, allocation_granularity: page }
}

pub(crate) fn memory_reserve(size: usize) -> Option<*mut u8> {
    // SAFETY: mapeamento anônimo novo; o resultado é checado.
    let p = unsafe {
        libc::mmap(
            null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
            -1,
            0,
        )
    };
    if p == libc::MAP_FAILED || p.is_null() { None } else { Some(p.cast::<u8>()) }
}

// ---------------------------------------------------------------- tempo

fn clock(id: libc::clockid_t) -> libc::timespec {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` é local e válido.
    unsafe { libc::clock_gettime(id, &mut ts) };
    ts
}

pub(crate) fn ticks() -> u64 {
    let ts = clock(libc::CLOCK_MONOTONIC);
    u64::try_from(ts.tv_sec)
        .unwrap_or(0)
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add(u64::try_from(ts.tv_nsec).unwrap_or(0))
}

pub(crate) fn ticks_per_second() -> u64 {
    NANOS_PER_SECOND
}

pub(crate) fn sleep_ms(ms: u32) {
    let mut request = libc::timespec {
        tv_sec: libc::time_t::from(ms.wrapping_div(1000)),
        tv_nsec: libc::c_long::from(ms.wrapping_rem(1000)).wrapping_mul(1_000_000),
    };
    let mut remaining = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: ponteiros para variáveis locais válidas. Repete se um sinal interromper.
    while unsafe { libc::nanosleep(&request, &mut remaining) } != 0 && errno() == libc::EINTR {
        request = remaining;
    }
}

pub(crate) fn begin_precise_sleep() {}

pub(crate) fn end_precise_sleep() {}

pub(crate) fn utc_now() -> UtcTime {
    let ts = clock(libc::CLOCK_REALTIME);
    // SAFETY: `tm` é só inteiros e um ponteiro (zero é válido); `gmtime_r` não aloca nem lê
    // fuso horário.
    let tm = unsafe {
        let mut tm: libc::tm = core::mem::zeroed();
        libc::gmtime_r(&ts.tv_sec, &mut tm);
        tm
    };
    UtcTime {
        year: u16::try_from(tm.tm_year.saturating_add(1900)).unwrap_or(0),
        month: u8::try_from(tm.tm_mon.saturating_add(1)).unwrap_or(0),
        day: u8::try_from(tm.tm_mday).unwrap_or(0),
        hour: u8::try_from(tm.tm_hour).unwrap_or(0),
        minute: u8::try_from(tm.tm_min).unwrap_or(0),
        second: u8::try_from(tm.tm_sec).unwrap_or(0),
    }
}

// ---------------------------------------------------------------- arquivos

pub(crate) type FileHandle = c_int;

pub(crate) fn file_create(path: &str) -> Option<FileHandle> {
    let mut buf = [0u8; PATH_MAX + 1];
    let p = c_path(path, &mut buf)?;
    // SAFETY: `p` termina em 0 e vive durante a chamada.
    let fd = unsafe {
        libc::open(
            p.as_ptr().cast::<c_char>(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
            0o644 as libc::c_uint,
        )
    };
    if fd < 0 { None } else { Some(fd) }
}

pub(crate) fn file_write_all(fd: &FileHandle, data: &[u8]) -> bool {
    let mut rest = data;
    while !rest.is_empty() {
        // SAFETY: `rest` tem `rest.len()` bytes válidos.
        let n = unsafe { libc::write(*fd, rest.as_ptr().cast::<c_void>(), rest.len()) };
        if n < 0 && errno() == libc::EINTR {
            continue;
        }
        let Ok(written) = usize::try_from(n) else {
            return false;
        };
        if written == 0 {
            return false;
        }
        let Some(r) = rest.get(written..) else {
            return false;
        };
        rest = r;
    }
    true
}

pub(crate) fn file_close(fd: FileHandle) {
    // SAFETY: `fd` veio de `file_create` e é fechado uma vez (o `FileWriter` foi consumido).
    unsafe { libc::close(fd) };
}

pub(crate) fn create_dir(path: &str) -> bool {
    let mut buf = [0u8; PATH_MAX + 1];
    let Some(p) = c_path(path, &mut buf) else {
        return false;
    };
    // SAFETY: `p` termina em 0. `errno` lido logo em seguida.
    unsafe { libc::mkdir(p.as_ptr().cast::<c_char>(), 0o755) == 0 || errno() == libc::EEXIST }
}

// ---------------------------------------------------------------- pilha

#[repr(C)]
struct UnwindContext {
    _private: [u8; 0],
}

const URC_NO_REASON: c_int = 0;
const URC_NORMAL_STOP: c_int = 4;

type UnwindTraceFn = extern "C" fn(*mut UnwindContext, *mut c_void) -> c_int;

// Da libgcc_s, que a std já linka no alvo linux-gnu.
unsafe extern "C" {
    fn _Unwind_Backtrace(trace: UnwindTraceFn, arg: *mut c_void) -> c_int;
    fn _Unwind_GetIP(ctx: *mut UnwindContext) -> usize;
}

struct Trace<'a> {
    frames: &'a mut [usize],
    count: usize,
    skip: u32,
}

extern "C" fn trace_frame(ctx: *mut UnwindContext, arg: *mut c_void) -> c_int {
    // SAFETY: `arg` é o `Trace` de `stack_capture`, vivo durante `_Unwind_Backtrace`.
    let t = unsafe { &mut *arg.cast::<Trace>() };
    // SAFETY: `ctx` vem do unwinder e vale durante esta chamada.
    let ip = unsafe { _Unwind_GetIP(ctx) };
    if ip == 0 {
        return URC_NORMAL_STOP;
    }
    if t.skip > 0 {
        t.skip = t.skip.saturating_sub(1);
        return URC_NO_REASON;
    }
    let Some(slot) = t.frames.get_mut(t.count) else {
        return URC_NORMAL_STOP;
    };
    *slot = ip;
    t.count = t.count.saturating_add(1);
    URC_NO_REASON
}

#[inline(always)]
pub(crate) fn stack_capture(skip: u32, frames: &mut [usize]) -> usize {
    // O primeiro quadro visto pelo unwinder é o de quem chamou `_Unwind_Backtrace` (esta função).
    let mut t = Trace { frames, count: 0, skip: skip.saturating_add(1) };
    // SAFETY: `t` vive até o fim da chamada; `trace_frame` só o acessa durante ela.
    unsafe { _Unwind_Backtrace(trace_frame, (&mut t as *mut Trace).cast::<c_void>()) };
    t.count
}

pub(crate) fn write_module_offset(t: &mut TextBuf, address: usize) {
    // SAFETY: `Dl_info` só tem ponteiros (zero é válido); `dladdr` só escreve nele.
    let (ok, info) = unsafe {
        let mut info: libc::Dl_info = core::mem::zeroed();
        let ok = libc::dladdr(address as *const c_void, &mut info);
        (ok, info)
    };
    if ok == 0 || info.dli_fname.is_null() {
        let _ = write!(t, "0x{address:x}");
        return;
    }
    // SAFETY: `dli_fname` é uma string C do carregador, válida enquanto o módulo estiver carregado.
    let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_bytes();
    let start = path.iter().rposition(|&c| c == b'/').map_or(0, |i| i.saturating_add(1));
    for chunk in path.get(start..).unwrap_or(&[]).utf8_chunks() {
        let _ = t.write_str(chunk.valid());
    }
    let _ = write!(t, "+0x{:x}", address.wrapping_sub(info.dli_fbase as usize));
}

// ---------------------------------------------------------------- erro fatal

const ALT_STACK_SIZE: usize = 64 * 1024;
/// Falha a menos disso do ponteiro de pilha é estouro de pilha (página de guarda).
const STACK_OVERFLOW_DISTANCE: usize = 64 * 1024;
const FPE_INTDIV: c_int = 1;
const FPE_INTOVF: c_int = 2;
const FATAL_SIGNALS: [c_int; 4] = [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE];

#[repr(C, align(16))]
struct AltStack(UnsafeCell<[u8; ALT_STACK_SIZE]>);

// SAFETY: só o SO usa este buffer, como pilha do handler de sinal da thread que chamou `install`.
unsafe impl Sync for AltStack {}

static ALT_STACK: AltStack = AltStack(UnsafeCell::new([0; ALT_STACK_SIZE]));
static HANDLER: AtomicPtr<()> = AtomicPtr::new(null_mut());

pub(crate) fn fatal_install(handler: fn(&FatalEvent)) {
    HANDLER.store(handler as *mut (), Ordering::Release);
    // SAFETY: pilha alternativa estática que nunca é liberada; `sigaction` com struct zerada e
    // preenchida aqui.
    unsafe {
        let stack = libc::stack_t { ss_sp: ALT_STACK.0.get().cast::<c_void>(), ss_flags: 0, ss_size: ALT_STACK_SIZE };
        libc::sigaltstack(&stack, null_mut());
        for sig in FATAL_SIGNALS {
            let mut action: libc::sigaction = core::mem::zeroed();
            action.sa_sigaction = signal_handler as *const () as usize;
            action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK | libc::SA_RESETHAND;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(sig, &action, null_mut());
        }
    }
}

fn installed_handler() -> Option<fn(&FatalEvent)> {
    let p = HANDLER.load(Ordering::Acquire);
    if p.is_null() {
        return None;
    }
    // SAFETY: o único valor não nulo guardado em HANDLER é um `fn(&FatalEvent)`.
    Some(unsafe { core::mem::transmute::<*mut (), fn(&FatalEvent)>(p) })
}

/// Registradores do contexto interrompido: (instrução, pilha, código de erro da falha).
#[cfg(target_arch = "x86_64")]
fn context_registers(ctx: *mut c_void) -> (usize, usize, usize) {
    if ctx.is_null() {
        return (0, 0, 0);
    }
    // SAFETY: com SA_SIGINFO o terceiro argumento do handler é um `ucontext_t` válido.
    let gregs = unsafe { &(*ctx.cast::<libc::ucontext_t>()).uc_mcontext.gregs };
    let reg = |r: c_int| usize::try_from(r).ok().and_then(|i| gregs.get(i)).map_or(0, |v| *v as usize);
    (reg(libc::REG_RIP), reg(libc::REG_RSP), reg(libc::REG_ERR))
}

#[cfg(not(target_arch = "x86_64"))]
fn context_registers(_ctx: *mut c_void) -> (usize, usize, usize) {
    (0, 0, 0)
}

fn kind_from_signal(sig: c_int, code: c_int, address: usize, sp: usize, err: usize) -> FatalKind {
    match sig {
        libc::SIGSEGV if sp != 0 && address.abs_diff(sp) < STACK_OVERFLOW_DISTANCE => FatalKind::StackOverflow,
        libc::SIGSEGV => FatalKind::AccessViolation {
            address,
            // Código de erro de page fault do x86: bit 1 = escrita, bit 4 = busca de instrução.
            access: if err & 0x10 != 0 {
                MemoryAccess::Execute
            } else if err & 0x2 != 0 {
                MemoryAccess::Write
            } else if cfg!(target_arch = "x86_64") {
                MemoryAccess::Read
            } else {
                MemoryAccess::Unknown
            },
        },
        libc::SIGBUS => FatalKind::BusError,
        libc::SIGILL => FatalKind::IllegalInstruction,
        libc::SIGFPE if code == FPE_INTDIV => FatalKind::DivideByZero,
        libc::SIGFPE if code == FPE_INTOVF => FatalKind::IntegerOverflow,
        _ => FatalKind::Other,
    }
}

extern "C" fn signal_handler(sig: c_int, info: *mut libc::siginfo_t, ctx: *mut c_void) {
    let (address, code) = if info.is_null() {
        (0, 0)
    } else {
        // SAFETY: com SA_SIGINFO o SO passa um `siginfo_t` válido.
        unsafe { ((*info).si_addr() as usize, (*info).si_code) }
    };
    let (ip, sp, err) = context_registers(ctx);
    let event = FatalEvent {
        kind: kind_from_signal(sig, code, address, sp, err),
        os_code: u32::try_from(sig).unwrap_or(0),
        instruction_address: ip,
    };
    if let Some(handler) = installed_handler() {
        handler(&event);
    }
    // Volta: com SA_RESETHAND a ação já é a padrão; a instrução repete e o sinal encerra o processo.
}

pub(crate) fn terminate(exit_code: u32) -> ! {
    // SAFETY: encerra o processo sem rodar nada em espaço de usuário.
    unsafe { libc::_exit(c_int::try_from(exit_code & 0xFF).unwrap_or(1)) }
}

// ---------------------------------------------------------------- bibliotecas dinâmicas

pub(crate) type LibraryHandle = *mut c_void;

pub(crate) fn library_open(name: &str) -> Option<LibraryHandle> {
    let mut buf = [0u8; NAME_MAX + 1];
    let n = c_path(name, &mut buf)?;
    // SAFETY: `n` termina em 0 e vive durante a chamada.
    let h = unsafe { libc::dlopen(n.as_ptr().cast::<c_char>(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if h.is_null() { None } else { Some(h) }
}

pub(crate) fn library_symbol(h: &LibraryHandle, name: &str) -> Option<*mut c_void> {
    let mut buf = [0u8; NAME_MAX + 1];
    let n = c_path(name, &mut buf)?;
    // SAFETY: `h` veio de dlopen; `n` termina em 0.
    let p = unsafe { libc::dlsym(*h, n.as_ptr().cast::<c_char>()) };
    if p.is_null() { None } else { Some(p) }
}

pub(crate) fn library_close(h: LibraryHandle) {
    // SAFETY: `h` veio de dlopen e é fechado uma vez (a `Library` foi consumida).
    unsafe { libc::dlclose(h) };
}

// ---------------------------------------------------------------- processo

pub(crate) fn thread_id() -> u64 {
    // SAFETY: chamada sem ponteiros.
    u64::try_from(unsafe { libc::gettid() }).unwrap_or(0)
}

// `c_ulong` é `u64` no Linux 64 bits, mas `u32` em 32 bits: a conversão fica por portabilidade.
#[allow(clippy::useless_conversion)]
pub(crate) fn system_memory() -> Option<SystemMemory> {
    // SAFETY: `sysinfo` só tem inteiros (zero é válido); a função só escreve.
    let si = unsafe {
        let mut si: libc::sysinfo = core::mem::zeroed();
        if libc::sysinfo(&mut si) != 0 {
            return None;
        }
        si
    };
    let unit = u64::from(si.mem_unit);
    Some(SystemMemory {
        total_physical: u64::from(si.totalram).saturating_mul(unit),
        available_physical: u64::from(si.freeram).saturating_add(u64::from(si.bufferram)).saturating_mul(unit),
        commit_limit: None,
        commit_available: None,
    })
}

/// Lê o segundo campo de `/proc/self/statm` (páginas residentes) sem alocar.
fn resident_pages() -> Option<u64> {
    // SAFETY: caminho é string C literal; `buf` é local.
    let (n, buf) = unsafe {
        let fd = libc::open(c"/proc/self/statm".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return None;
        }
        let mut buf = [0u8; 128];
        let n = libc::read(fd, buf.as_mut_ptr().cast::<c_void>(), buf.len());
        libc::close(fd);
        (n, buf)
    };
    let text = buf.get(..usize::try_from(n).ok()?)?;
    let field = text.split(|&b| b == b' ').nth(1)?;
    parse_u64(field)
}

fn parse_u64(digits: &[u8]) -> Option<u64> {
    let mut v: u64 = 0;
    let mut any = false;
    for &c in digits {
        if !c.is_ascii_digit() {
            break;
        }
        v = v.checked_mul(10)?.checked_add(u64::from(c.wrapping_sub(b'0')))?;
        any = true;
    }
    if any { Some(v) } else { None }
}

pub(crate) fn process_memory() -> Option<ProcessMemory> {
    let page = page_info().page_size as u64;
    let resident = resident_pages()?.saturating_mul(page);
    // SAFETY: `rusage` só tem inteiros (zero é válido); a função só escreve.
    let peak_kb = unsafe {
        let mut usage: libc::rusage = core::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
            return None;
        }
        usage.ru_maxrss
    };
    Some(ProcessMemory {
        resident,
        peak_resident: u64::try_from(peak_kb).unwrap_or(0).saturating_mul(1024),
        committed: None,
        peak_committed: None,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // testes seguem as mesmas exceções do clippy.toml
mod tests {
    use super::*;

    #[test]
    fn c_path_is_null_terminated_and_respects_buffer() {
        let mut buf = [0xFFu8; 4];
        assert_eq!(c_path("ab", &mut buf), Some(&b"ab\0"[..]));
        assert_eq!(c_path("abcd", &mut buf), None);
    }

    #[test]
    fn parses_statm_numbers() {
        assert_eq!(parse_u64(b"1234 "), Some(1234));
        assert_eq!(parse_u64(b"x"), None);
    }

    #[test]
    fn signals_map_to_fatal_kinds() {
        assert_eq!(
            kind_from_signal(libc::SIGSEGV, 0, 0, 0x7fff_0000, 0x6),
            FatalKind::AccessViolation { address: 0, access: MemoryAccess::Write }
        );
        assert_eq!(kind_from_signal(libc::SIGSEGV, 0, 0x7fff_0000 - 8, 0x7fff_0000, 0x6), FatalKind::StackOverflow);
        assert_eq!(kind_from_signal(libc::SIGFPE, FPE_INTDIV, 0, 0, 0), FatalKind::DivideByZero);
    }
}
