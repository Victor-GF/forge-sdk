//! Crash report.
//!
//! Entradas:
//! - `crash::fatal(reason, details)`: erro fatal decidido pelo código (OOM de arena, heap
//!   proibido...). Grava o relatório e aborta.
//! - panic: o hook instalado por `crash::install` grava e aborta antes de a std alocar o
//!   payload do unwind.
//! - erros do SO (access violation, stack overflow, instrução ilegal...): handler de erro
//!   fatal do `forja_os` (exceções no Windows, sinais no Linux).
//!
//! `abort()` não passa pelo handler de erro fatal (no Windows usa *fast fail*); por isso todo
//! caminho que aborta de propósito passa por `crash::fatal` antes.
//!
//! O relatório é montado num buffer estático (`REPORT_CAPACITY`) e gravado pelo `forja_os`:
//! nada é alocado, então funciona sem memória livre e com o heap geral proibido.
//! Pouca stack é usada, porque no stack overflow o handler roda numa reserva pequena.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU64, Ordering};

use forja_core::text::TextBuf;
use forja_os::fatal::{self, FatalEvent, FatalKind, MemoryAccess};
use forja_os::file::{self, FileWriter};
use forja_os::{process, stack, time};

use crate::log::write_breadcrumbs;
use crate::spin_lock::SpinLock;

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
/// Código de saída quando um crash acontece dentro do próprio relatório.
const NESTED_CRASH_EXIT_CODE: u32 = 0xDEAD;

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
static START_TICKS: AtomicU64 = AtomicU64::new(0);
static FRAME: AtomicU64 = AtomicU64::new(0);
/// Thread que está escrevendo o relatório (0 = nenhuma).
static CRASH_THREAD: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------- API

/// Instala o hook de panic e o handler de erro fatal do SO. Chame no início do `main`, antes
/// de `global::end_startup()`.
pub fn install(config: CrashConfig) {
    if SHARED.lock.try_lock(u32::MAX) {
        // SAFETY: lock pego.
        unsafe { (*SHARED.state.get()).config = config };
        SHARED.lock.unlock();
    }
    START_TICKS.store(time::ticks(), Ordering::Relaxed);
    fatal::install(on_fatal);
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

fn fatal_reason(kind: FatalKind) -> &'static str {
    match kind {
        FatalKind::AccessViolation { .. } => "access violation",
        FatalKind::StackOverflow => "stack overflow",
        FatalKind::IllegalInstruction => "instrução ilegal",
        FatalKind::DivideByZero => "divisão inteira por zero",
        FatalKind::IntegerOverflow => "overflow inteiro",
        FatalKind::BusError => "erro de barramento/página",
        FatalKind::Breakpoint => "breakpoint",
        FatalKind::Other => "erro fatal do SO",
    }
}

fn access_verb(access: MemoryAccess) -> &'static str {
    match access {
        MemoryAccess::Read => "ler",
        MemoryAccess::Write => "escrever",
        MemoryAccess::Execute => "executar",
        MemoryAccess::Unknown => "acessar",
    }
}

fn on_fatal(e: &FatalEvent) {
    let reason = fatal_reason(e.kind);
    match e.kind {
        FatalKind::AccessViolation { address, access } => write_report(
            reason,
            format_args!(
                "código do SO 0x{:08X}: tentou {} o endereço 0x{address:016x}",
                e.os_code,
                access_verb(access)
            ),
            Some(e.instruction_address),
        ),
        _ => write_report(reason, format_args!("código do SO 0x{:08X}", e.os_code), Some(e.instruction_address)),
    }
}

// ---------------------------------------------------------------- relatório

/// Garante que só uma thread escreve o relatório. Crash dentro do crash (mesma thread)
/// encerra na hora; outras threads esperam o processo acabar.
fn enter_crash() -> bool {
    let me = process::thread_id();
    match CRASH_THREAD.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => true,
        Err(owner) if owner == me => fatal::terminate(NESTED_CRASH_EXIT_CODE),
        Err(_) => loop {
            time::sleep_ms(1000);
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

fn write_report(reason: &str, details: fmt::Arguments, instruction_address: Option<usize>) {
    if !enter_crash() {
        return;
    }
    let state = read_state();
    // SAFETY: só esta thread chega aqui (`enter_crash`).
    let buf: &mut [u8] = unsafe { &mut *REPORT.0.get() };
    let mut t = TextBuf::new(buf);

    let now = time::utc_now();
    let elapsed = time::ticks().saturating_sub(START_TICKS.load(Ordering::Relaxed));
    let uptime_s = time::ticks_to_seconds(elapsed, time::ticks_per_second());

    let _ = writeln!(t, "=== Forja: relatório de crash ===");
    let _ = writeln!(t, "jogo:    {} {}", state.config.app, state.config.version);
    let _ = writeln!(t, "sistema: {}", forja_os::CURRENT.name());
    let _ = writeln!(
        t,
        "data:    {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        now.year, now.month, now.day, now.hour, now.minute, now.second
    );
    let _ = writeln!(t, "rodando: {uptime_s:.3} s");
    let _ = writeln!(t, "frame:   {}", FRAME.load(Ordering::Relaxed));
    let _ = writeln!(t, "thread:  {}", process::thread_id());
    let _ = writeln!(t, "motivo:  {reason}");
    let _ = writeln!(t, "detalhe: {details}");

    write_stack(&mut t, instruction_address);
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
        state.config.dir, now.year, now.month, now.day, now.hour, now.minute, now.second
    );
    let _ = file::create_dir(state.config.dir);
    let written = match FileWriter::create(path.as_str()) {
        Some(f) => {
            let ok = f.write_all(t.as_bytes());
            f.close();
            ok
        }
        None => false,
    };

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

fn write_stack(t: &mut TextBuf, instruction_address: Option<usize>) {
    let _ = writeln!(t, "\n--- pilha (módulo+deslocamento; simbolize com tools/symbolize.sh) ---");
    if let Some(address) = instruction_address {
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
    if let Some(m) = process::system_memory() {
        let _ = writeln!(
            t,
            "RAM:      {:.0} MB livres de {:.0} MB",
            mib(m.available_physical),
            mib(m.total_physical)
        );
        if let (Some(limit), Some(available)) = (m.commit_limit, m.commit_available) {
            let _ = writeln!(t, "commit:   {:.0} MB livres de {:.0} MB (limite do sistema)", mib(available), mib(limit));
        }
    }
    if let Some(p) = process::process_memory() {
        let _ = write!(t, "processo: residente {:.1} MB (pico {:.1})", mib(p.resident), mib(p.peak_resident));
        if let (Some(c), Some(pc)) = (p.committed, p.peak_committed) {
            let _ = write!(t, ", commit {:.1} MB (pico {:.1})", mib(c), mib(pc));
        }
        let _ = t.write_char('\n');
    }
}
