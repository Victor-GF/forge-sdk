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
//! O arquivo só tem campos fixos, como o relatório de um build Shipping da Unreal: motivo,
//! código do SO, tipo de acesso à memória, pilha em módulo+deslocamento e memória do sistema.
//! Nada de texto montado em tempo de execução (mensagem de panic, detalhes, log) nem endereço
//! absoluto: o arquivo pode sair da máquina do jogador sem levar dados dele. O texto livre vai
//! só para o stderr. A pilha é simbolizada fora (`tools/symbolize.sh`), com o binário do mesmo
//! build guardado por quem publicou (o papel dos PDBs na Unreal); a linha `build:` (build id
//! do executável) diz qual é. Ver "Release e símbolos" em `docs/style.md`.
//!
//! O relatório é montado num buffer estático (`REPORT_CAPACITY`) e gravado pelo `forja_os`:
//! nada é alocado, então funciona sem memória livre.
//! Pouca stack é usada, porque no stack overflow o handler roda numa reserva pequena.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU64, Ordering};

use forja_core::text::TextBuf;
use forja_os::fatal::{self, FatalEvent, FatalKind, MemoryAccess};
use forja_os::file::{self, FileWriter};
use forja_os::stack::BuildId;
use forja_os::{process, stack, time};

use crate::log;
use crate::spin_lock::SpinLock;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrashConfig {
    pub app: &'static str,
    pub version: &'static str,
    /// Pasta dos relatórios (um nível, criada se não existir).
    pub dir: &'static str,
}

const DEFAULT_CONFIG: CrashConfig = CrashConfig { app: "?", version: "?", dir: "crash" };

pub const REPORT_CAPACITY: usize = 16 * 1024;
/// Código de saída quando um crash acontece dentro do próprio relatório.
const NESTED_CRASH_EXIT_CODE: u32 = 0xDEAD;
/// Acesso abaixo disso é ponteiro nulo (mais um campo): o SO nunca mapeia esses endereços
/// (64 KB no Windows e no `vm.mmap_min_addr` padrão do Linux).
const NULL_PAGE_LIMIT: usize = 64 * 1024;

/// O que `install` guarda para o relatório.
#[derive(Clone, Copy)]
struct Installed {
    config: CrashConfig,
    /// Lido na instalação: no crash só é copiado.
    build_id: Option<BuildId>,
}

struct Shared {
    lock: SpinLock,
    installed: UnsafeCell<Installed>,
}

// SAFETY: `installed` só é escrito com `lock` pego; no crash é lido mesmo sem ele (melhor esforço).
unsafe impl Sync for Shared {}

static SHARED: Shared = Shared {
    lock: SpinLock::new(),
    installed: UnsafeCell::new(Installed { config: DEFAULT_CONFIG, build_id: None }),
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

/// Instala o hook de panic e o handler de erro fatal do SO. Chame no início do `main`.
pub fn install(config: CrashConfig) {
    let build_id = stack::build_id();
    if SHARED.lock.try_lock(u32::MAX) {
        // SAFETY: lock pego.
        unsafe { *SHARED.installed.get() = Installed { config, build_id } };
        SHARED.lock.unlock();
    }
    // No log também: um log com pilhas (módulo+deslocamento) é simbolizado por ele.
    let mut id_buf = [0u8; 2 * stack::BUILD_ID_MAX];
    let mut id_text = TextBuf::new(&mut id_buf);
    write_build_id(&mut id_text, build_id);
    log::info(format_args!("crash report ativo; build: {}", id_text.as_str()));
    START_TICKS.store(time::ticks(), Ordering::Relaxed);
    fatal::install(on_fatal);
    // `panic_hook` é uma função (tamanho zero): o Box não aloca.
    std::panic::set_hook(Box::new(panic_hook));
}

/// Frame atual, para o relatório. Barato: um store atômico.
pub fn set_frame(index: u64) {
    FRAME.store(index, Ordering::Relaxed);
}

/// Erro fatal: grava o relatório e aborta. `reason` é um literal e vai no arquivo; `details`
/// é texto livre e vai só para o stderr.
pub fn fatal(reason: &'static str, details: fmt::Arguments) -> ! {
    write_report(&Crash { reason, details: Some(details), os: None });
    std::process::abort()
}

/// O que aconteceu, separado pelo que pode ir no arquivo.
struct Crash<'a> {
    /// Texto fixo do código: vai no arquivo.
    reason: &'static str,
    /// Texto livre (mensagem do panic, valores do erro): só no stderr.
    details: Option<fmt::Arguments<'a>>,
    /// Erro vindo do SO: código, instrução e acesso à memória.
    os: Option<&'a FatalEvent>,
}

// ---------------------------------------------------------------- entradas do SO e da std

fn panic_hook(info: &std::panic::PanicHookInfo) {
    let msg = info.payload_as_str().unwrap_or("(mensagem não textual)");
    match info.location() {
        Some(l) => write_report(&Crash {
            reason: "panic",
            details: Some(format_args!("{msg} (em {}:{}:{})", l.file(), l.line(), l.column())),
            os: None,
        }),
        None => write_report(&Crash { reason: "panic", details: Some(format_args!("{msg}")), os: None }),
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
    write_report(&Crash { reason: fatal_reason(e.kind), details: None, os: Some(e) });
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

fn read_installed() -> Installed {
    let locked = SHARED.lock.try_lock(1000);
    // SAFETY: com o lock, ou no crash (melhor esforço).
    let c = unsafe { *SHARED.installed.get() };
    if locked {
        SHARED.lock.unlock();
    }
    c
}

fn write_report(crash: &Crash) {
    if !enter_crash() {
        return;
    }
    let Installed { config, build_id } = read_installed();
    // SAFETY: só esta thread chega aqui (`enter_crash`).
    let buf: &mut [u8] = unsafe { &mut *REPORT.0.get() };
    let mut t = TextBuf::new(buf);

    let now = time::utc_now();
    let elapsed = time::ticks().saturating_sub(START_TICKS.load(Ordering::Relaxed));
    let uptime_s = time::ticks_to_seconds(elapsed, time::ticks_per_second());

    let _ = writeln!(t, "=== Forja: relatório de crash ===");
    let _ = writeln!(t, "jogo:    {} {}", config.app, config.version);
    let _ = writeln!(t, "sistema: {}", forja_os::CURRENT.name());
    let _ = t.write_str("build:   ");
    write_build_id(&mut t, build_id);
    let _ = t.write_char('\n');
    let _ = writeln!(
        t,
        "data:    {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        now.year, now.month, now.day, now.hour, now.minute, now.second
    );
    let _ = writeln!(t, "rodando: {uptime_s:.3} s");
    let _ = writeln!(t, "frame:   {}", FRAME.load(Ordering::Relaxed));
    let _ = writeln!(t, "motivo:  {}", crash.reason);
    if let Some(e) = crash.os {
        let _ = writeln!(t, "erro SO: 0x{:08X}", e.os_code);
        if let FatalKind::AccessViolation { address, access } = e.kind {
            let target = if address < NULL_PAGE_LIMIT { "endereço nulo" } else { "endereço inválido" };
            let _ = writeln!(t, "acesso:  {} em {target}", access_verb(access));
        }
    }
    write_stack(&mut t, crash.os.map(|e| e.instruction_address));
    write_system_memory(&mut t);
    if t.truncated() {
        let _ = t.write_str("\n[relatório cortado: buffer cheio]\n");
    }

    let mut path_buf = [0u8; 512];
    let mut path = TextBuf::new(&mut path_buf);
    let _ = write!(
        path,
        "{}/crash_{:04}-{:02}-{:02}_{:02}{:02}{:02}.txt",
        config.dir, now.year, now.month, now.day, now.hour, now.minute, now.second
    );
    let _ = file::create_dir(config.dir);
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
    match crash.details {
        Some(details) => {
            let _ = writeln!(notice, "[crash] {}: {details}", crash.reason);
        }
        None => {
            let _ = writeln!(notice, "[crash] {}", crash.reason);
        }
    }
    if written {
        let _ = writeln!(notice, "[crash] relatório em {}", path.as_str());
    } else {
        let _ = writeln!(notice, "[crash] falha ao gravar o relatório em {}", path.as_str());
    }
    let _ = std::io::Write::write_all(&mut std::io::stderr(), notice.as_bytes());
}

/// Só módulo+deslocamento: o endereço absoluto não diz nada fora do processo. Fora de módulo
/// vira `?`.
fn write_stack(t: &mut TextBuf, instruction_address: Option<usize>) {
    let _ = writeln!(t, "\n--- pilha (módulo+deslocamento; simbolize com tools/symbolize.sh) ---");
    if let Some(address) = instruction_address {
        let _ = t.write_str("exceção ");
        write_frame(t, address);
    }
    let mut frames = [0usize; stack::FRAMES_MAX];
    let n = stack::capture(1, &mut frames);
    for (i, f) in frames.iter().take(n).enumerate() {
        let _ = write!(t, "#{i:02}     ");
        write_frame(t, *f);
    }
}

/// Hex do build id, ou `desconhecido` (sem ele, a pilha só pode ser simbolizada com o binário
/// certo escolhido à mão).
fn write_build_id(t: &mut TextBuf, build_id: Option<BuildId>) {
    match build_id {
        Some(id) => id.write_hex(t),
        None => {
            let _ = t.write_str("desconhecido");
        }
    }
}

fn write_frame(t: &mut TextBuf, address: usize) {
    if !stack::write_module_offset(t, address) {
        let _ = t.write_char('?');
    }
    let _ = t.write_char('\n');
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
