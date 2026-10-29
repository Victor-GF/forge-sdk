//! Kit de falhas: a engine parte do princípio de que algo vai dar errado.
//!
//! - Crash nativo (access violation, stack overflow): filtro SEH -> thread dedicada
//!   grava minidump + relatório (a thread que falhou pode estar sem pilha).
//! - Panic em sistema: o scheduler aplica a política declarada; "abortar" chega aqui.
//! - Travamento: watchdog observa o heartbeat do frame.
//! - Falhas suaves (invariantes, spikes): relatórios leves, com limite de frequência.
//!
//! Todo relatório leva o contexto do scheduler (o que cada lane executava), o flight
//! recorder do profiler e o log de input para reproduzir a partida com `--replay`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, RwLock};
use std::time::{Duration, Instant};

use crate::platform::{self, HANDLE};
use crate::profiler;

#[link(name = "kernel32")]
extern "system" {
    fn SetUnhandledExceptionFilter(f: Option<unsafe extern "system" fn(*mut c_void) -> i32>) -> *mut c_void;
    fn CreateEventW(attr: *mut c_void, manual: i32, initial: i32, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> i32;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "dbghelp")]
extern "system" {
    fn MiniDumpWriteDump(
        process: HANDLE, pid: u32, file: HANDLE, kind: u32, exc: *const MinidumpExceptionInfo,
        user: *const c_void, callback: *const c_void,
    ) -> i32;
}

#[repr(C, packed(4))]
struct MinidumpExceptionInfo {
    thread_id: u32,
    exception_pointers: *mut c_void,
    client_pointers: i32,
}

#[repr(C)]
struct ExceptionPointers {
    record: *mut ExceptionRecord,
    context: *mut c_void,
}

#[repr(C)]
struct ExceptionRecord {
    code: u32,
    flags: u32,
    record: *mut ExceptionRecord,
    address: *mut c_void,
    nparams: u32,
    info: [usize; 15],
}

const MAX_LANES: usize = 64;
// MiniDumpWithDataSegs | MiniDumpWithIndirectlyReferencedMemory | MiniDumpWithThreadInfo
const DUMP_KIND: u32 = 0x0001 | 0x0040 | 0x1000;

pub static WATCHDOG_PAUSED: AtomicBool = AtomicBool::new(false);
static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static TICK: AtomicU64 = AtomicU64::new(0);
static LANE_SYS: [AtomicU32; MAX_LANES] = [const { AtomicU32::new(0) }; MAX_LANES];
static LANE_TID: [AtomicU32; MAX_LANES] = [const { AtomicU32::new(0) }; MAX_LANES];
static NAMES: RwLock<Vec<(&'static str, &'static str)>> = RwLock::new(Vec::new());
static DISABLED: Mutex<Vec<u16>> = Mutex::new(Vec::new());
static BREADCRUMBS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static FAILURES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static REPORTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static RATE: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());
static INPUT: Mutex<InputLog> = Mutex::new(InputLog { inputs: Vec::new(), hashes: Vec::new() });
static CONFIG: OnceLock<Config> = OnceLock::new();
static REPORT_SEQ: AtomicU32 = AtomicU32::new(0);
static FATAL: AtomicBool = AtomicBool::new(false);

static REQ_EVT: AtomicUsize = AtomicUsize::new(0);
static DONE_EVT: AtomicUsize = AtomicUsize::new(0);
static SEH_FIRED: AtomicBool = AtomicBool::new(false);
static EXC_PTRS: AtomicUsize = AtomicUsize::new(0);
static EXC_TID: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static IN_SYSTEM: Cell<u32> = const { Cell::new(0) };
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub struct Config {
    pub dir: PathBuf,
    pub app: &'static str,
    pub interactive: bool,
}

#[derive(Clone, Debug, Default)]
pub struct InputLog {
    pub inputs: Vec<u32>,
    pub hashes: Vec<(u64, u64)>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lock com prazo: durante um crash a thread que falhou pode estar segurando o mutex.
fn lock_for<T>(m: &Mutex<T>, ms: u64) -> Option<MutexGuard<'_, T>> {
    let until = Instant::now() + Duration::from_millis(ms);
    loop {
        match m.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(_) if Instant::now() >= until => return None,
            Err(_) => std::thread::yield_now(),
        }
    }
}

// ---------------------------------------------------------------- instalação

pub fn install(dir: impl Into<PathBuf>, app: &'static str, interactive: bool) {
    let dir = dir.into();
    let _ = std::fs::create_dir_all(&dir);
    let _ = CONFIG.set(Config { dir, app, interactive });

    std::panic::set_hook(Box::new(|info| {
        let msg = payload_str(info.payload());
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("{}\n  em {}\n\nBacktrace da thread {:?}:\n{}", msg, loc, std::thread::current().name().unwrap_or("?"), bt);
        if IN_SYSTEM.with(|c| c.get()) > 0 {
            eprintln!("[forja] panic dentro de sistema: {} ({})", msg, loc);
            LAST_PANIC.with(|p| *p.borrow_mut() = Some(text));
        } else {
            eprintln!("[forja] panic fora de qualquer sistema: {} ({})", msg, loc);
            write_report("panic", "panic fora de um sistema", &text, true);
        }
    }));

    unsafe {
        let req = CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null());
        let done = CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null());
        REQ_EVT.store(req as usize, Ordering::SeqCst);
        DONE_EVT.store(done as usize, Ordering::SeqCst);
        SetUnhandledExceptionFilter(Some(seh_filter));
    }
    std::thread::Builder::new().name("crash-handler".into()).spawn(handler_thread).expect("crash-handler");
    std::thread::Builder::new().name("watchdog".into()).spawn(watchdog_thread).expect("watchdog");
}

pub fn register_systems(base: u16, names: &[(&'static str, &'static str)]) {
    let mut v = NAMES.write().unwrap_or_else(|e| e.into_inner());
    let end = base as usize + names.len();
    if v.len() < end {
        v.resize(end, ("?", "?"));
    }
    v[base as usize..end].copy_from_slice(names);
}

pub fn register_lane_thread(lane: usize) {
    if lane < MAX_LANES {
        LANE_TID[lane].store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- contexto em runtime

pub fn heartbeat(frame: u64) {
    HEARTBEAT.store(frame, Ordering::Relaxed);
}

pub fn set_tick(t: u64) {
    TICK.store(t, Ordering::Relaxed);
}

pub fn frame() -> u64 {
    HEARTBEAT.load(Ordering::Relaxed)
}

pub fn tick() -> u64 {
    TICK.load(Ordering::Relaxed)
}

/// Marca que a lane está executando o sistema `id`. Devolve o valor anterior.
pub fn enter_system(lane: usize, id: u16) -> u32 {
    IN_SYSTEM.with(|c| c.set(c.get() + 1));
    if lane < MAX_LANES {
        LANE_SYS[lane].swap(id as u32 + 1, Ordering::Relaxed)
    } else {
        0
    }
}

pub fn leave_system(lane: usize, prev: u32) {
    IN_SYSTEM.with(|c| c.set(c.get().saturating_sub(1)));
    if lane < MAX_LANES {
        LANE_SYS[lane].store(prev, Ordering::Relaxed);
    }
}

pub fn take_panic_text() -> Option<String> {
    LAST_PANIC.with(|p| p.borrow_mut().take())
}

pub fn payload_str(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "(payload de panic desconhecido)".into()
    }
}

pub fn breadcrumb(msg: impl Into<String>) {
    let s = format!("[f{} t{}] {}", frame(), tick(), msg.into());
    let mut b = lock(&BREADCRUMBS);
    if b.len() >= 64 {
        b.pop_front();
    }
    b.push_back(s);
}

pub fn set_disabled(id: u16, disabled: bool) {
    let mut d = lock(&DISABLED);
    d.retain(|&x| x != id);
    if disabled {
        d.push(id);
    }
}

pub fn breadcrumbs() -> Vec<String> {
    lock(&BREADCRUMBS).iter().cloned().collect()
}
pub fn failures() -> Vec<String> {
    lock(&FAILURES).iter().cloned().collect()
}
pub fn reports() -> Vec<String> {
    lock(&REPORTS).clone()
}

fn sys_label(v: u32) -> String {
    if v == 0 {
        return "(ocioso / fora do grafo)".into();
    }
    match NAMES.try_read() {
        Ok(n) => n.get(v as usize - 1).map(|(s, p)| format!("{} [{}]", s, p)).unwrap_or_else(|| format!("sistema #{}", v - 1)),
        Err(_) => format!("sistema #{}", v - 1),
    }
}

pub fn lane_name(l: usize) -> String {
    if l == 0 {
        "main".into()
    } else {
        format!("worker-{}", l)
    }
}

// ---------------------------------------------------------------- log de input / replay

pub fn record_input(tick: u64, bits: u32) {
    let mut l = lock(&INPUT);
    if l.inputs.len() as u64 == tick {
        l.inputs.push(bits);
    }
}

pub fn record_hash(tick: u64, h: u64) {
    lock(&INPUT).hashes.push((tick, h));
}

pub fn input_log() -> InputLog {
    lock(&INPUT).clone()
}

const LOG_MAGIC: &[u8; 8] = b"FORJALG1";

fn encode_log(l: &InputLog) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + l.inputs.len() * 4 + l.hashes.len() * 16);
    out.extend_from_slice(LOG_MAGIC);
    out.extend_from_slice(&(l.inputs.len() as u32).to_le_bytes());
    out.extend_from_slice(&(l.hashes.len() as u32).to_le_bytes());
    for i in &l.inputs {
        out.extend_from_slice(&i.to_le_bytes());
    }
    for (t, h) in &l.hashes {
        out.extend_from_slice(&t.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
    }
    out
}

pub fn save_input_log(path: &Path) -> std::io::Result<()> {
    let bytes = match lock_for(&INPUT, 200) {
        Some(l) => encode_log(&l),
        None => return Err(std::io::Error::other("log de input ocupado")),
    };
    std::fs::write(path, bytes)
}

pub fn load_input_log(path: &Path) -> std::io::Result<InputLog> {
    let mut f = std::fs::File::open(path)?;
    let mut b = Vec::new();
    f.read_to_end(&mut b)?;
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "log de input inválido");
    if b.len() < 16 || &b[..8] != LOG_MAGIC {
        return Err(bad());
    }
    let ni = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
    let nh = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
    if b.len() < 16 + ni * 4 + nh * 16 {
        return Err(bad());
    }
    let mut p = 16;
    let mut log = InputLog::default();
    for _ in 0..ni {
        log.inputs.push(u32::from_le_bytes(b[p..p + 4].try_into().unwrap()));
        p += 4;
    }
    for _ in 0..nh {
        let t = u64::from_le_bytes(b[p..p + 8].try_into().unwrap());
        let h = u64::from_le_bytes(b[p + 8..p + 16].try_into().unwrap());
        log.hashes.push((t, h));
        p += 16;
    }
    Ok(log)
}

// ---------------------------------------------------------------- relatórios

fn rate_ok(key: &str, every: Duration) -> bool {
    let mut r = lock(&RATE);
    let now = Instant::now();
    if let Some(e) = r.iter_mut().find(|(k, _)| k == key) {
        if now.duration_since(e.1) < every {
            return false;
        }
        e.1 = now;
        return true;
    }
    r.push((key.to_string(), now));
    true
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ã' | 'â' | 'Á' | 'Ã' | 'Â' => 'a',
            'é' | 'ê' | 'É' | 'Ê' => 'e',
            'í' | 'Í' => 'i',
            'ó' | 'õ' | 'ô' | 'Ó' | 'Õ' | 'Ô' => 'o',
            'ú' | 'Ú' => 'u',
            'ç' | 'Ç' => 'c',
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').chars().take(40).collect()
}

fn new_report_dir(kind: &str, title: &str) -> Option<PathBuf> {
    let cfg = CONFIG.get()?;
    let t = platform::local_time();
    let seq = REPORT_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}_{:02}_{}_{}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, seq, kind, slug(title)
    );
    let dir = cfg.dir.join(name);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Grava um relatório completo. Usa locks com prazo: pode ser chamado durante um crash.
pub fn write_report(kind: &str, title: &str, details: &str, dump: bool) -> Option<PathBuf> {
    let cfg = CONFIG.get()?;
    let dir = new_report_dir(kind, title)?;
    let t = platform::local_time();
    let mut r = String::new();
    let _ = writeln!(r, "=== RELATÓRIO DE FALHA: {} ===", cfg.app);
    let _ = writeln!(r, "tipo:      {}", kind);
    let _ = writeln!(r, "título:    {}", title);
    let _ = writeln!(r, "quando:    {:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    let _ = writeln!(
        r,
        "build:     {} {} ({})",
        cfg.app,
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );
    let _ = writeln!(r, "processo:  pid {}", unsafe { GetCurrentProcessId() });
    let _ = writeln!(r, "frame:     {}    tick de simulação: {}", frame(), tick());

    let _ = writeln!(r, "\n--- O que cada lane estava executando ---");
    for l in 0..MAX_LANES {
        let tid = LANE_TID[l].load(Ordering::Relaxed);
        if tid == 0 {
            continue;
        }
        let _ = writeln!(r, "{:<10} (tid {:>6}): {}", lane_name(l), tid, sys_label(LANE_SYS[l].load(Ordering::Relaxed)));
    }

    if let Some(d) = lock_for(&DISABLED, 50) {
        if !d.is_empty() {
            let _ = writeln!(r, "\n--- Sistemas desligados por falha ---");
            for id in d.iter() {
                let _ = writeln!(r, "  {}", sys_label(*id as u32 + 1));
            }
        }
    }
    if let Some(f) = lock_for(&FAILURES, 50) {
        if !f.is_empty() {
            let _ = writeln!(r, "\n--- Falhas anteriores nesta sessão ---");
            for s in f.iter() {
                let _ = writeln!(r, "  {}", s);
            }
        }
    }
    if let Some(b) = lock_for(&BREADCRUMBS, 50) {
        let _ = writeln!(r, "\n--- Breadcrumbs (últimos eventos) ---");
        for s in b.iter() {
            let _ = writeln!(r, "  {}", s);
        }
    }
    let _ = writeln!(r, "\n--- Detalhes ---\n{}", details);
    let _ = writeln!(r, "\n--- Como investigar ---");
    let _ = writeln!(r, "flight_recorder.json  últimos segundos de profiling: abra em https://ui.perfetto.dev");
    let _ = writeln!(r, "input.log             reproduz a partida até aqui:");
    let _ = writeln!(r, "                      cargo run --release -p corrida -- --replay \"{}\"", dir.join("input.log").display());
    if dump {
        let _ = writeln!(r, "dump.dmp              minidump: abra no Visual Studio ou WinDbg (os .pdb ficam em target/)");
    }

    let _ = std::fs::write(dir.join("report.txt"), r);
    let _ = save_input_log(&dir.join("input.log"));
    if let Some(json) = profiler::get().export_chrome(6.0, true, &lane_name) {
        let _ = std::fs::write(dir.join("flight_recorder.json"), json);
    }
    if dump {
        write_dump(&dir.join("dump.dmp"), None);
    }
    if let Some(mut rep) = lock_for(&REPORTS, 50) {
        rep.push(dir.display().to_string());
    }
    eprintln!("[forja] relatório '{}' gravado em {}", kind, dir.display());
    Some(dir)
}

fn write_dump(path: &Path, exc: Option<(u32, usize)>) -> bool {
    let Ok(file) = std::fs::File::create(path) else { return false };
    let info = exc.map(|(tid, ptrs)| MinidumpExceptionInfo { thread_id: tid, exception_pointers: ptrs as *mut c_void, client_pointers: 0 });
    let ok = unsafe {
        MiniDumpWriteDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            file.as_raw_handle() as HANDLE,
            DUMP_KIND,
            info.as_ref().map_or(std::ptr::null(), |i| i as *const _),
            std::ptr::null(),
            std::ptr::null(),
        ) != 0
    };
    let _ = (&file).flush();
    ok
}

/// Falha que não dá para contornar: relatório completo + minidump, e encerra.
pub fn fatal(title: &str, details: &str) -> ! {
    if FATAL.swap(true, Ordering::SeqCst) {
        // outra thread já está reportando; espera o processo encerrar
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    breadcrumb(format!("FATAL: {}", title));
    let dir = write_report("crash", title, details, true);
    if CONFIG.get().is_some_and(|c| c.interactive) {
        let where_ = dir.map(|d| d.display().to_string()).unwrap_or_else(|| "(falha ao gravar)".into());
        platform::message_box("forja: falha crítica", &format!("{}\n\nRelatório, minidump e replay em:\n{}", title, where_));
    }
    std::process::exit(3);
}

/// Falha recuperável de um sistema (a política já foi aplicada pelo scheduler).
pub fn system_failure(system: &str, policy: &str, msg: &str) {
    let first = msg.lines().next().unwrap_or("").to_string();
    let line = format!("[f{} t{}] {} -> {}: {}", frame(), tick(), system, policy, first);
    {
        let mut f = lock(&FAILURES);
        if f.len() >= 32 {
            f.pop_front();
        }
        f.push_back(line);
    }
    breadcrumb(format!("{} falhou ({})", system, policy));
    if rate_ok(system, Duration::from_secs(10)) {
        write_report("falha", &format!("{} falhou", system), &format!("política aplicada: {}\n\n{}", policy, msg), false);
    }
}

pub fn spike(details: &str) {
    if rate_ok("spike", Duration::from_secs(15)) {
        write_report("spike", "frame acima do orçamento", details, false);
    }
}

// ---------------------------------------------------------------- crash nativo (SEH)

unsafe extern "system" fn seh_filter(ptrs: *mut c_void) -> i32 {
    if SEH_FIRED.swap(true, Ordering::SeqCst) {
        return 0; // EXCEPTION_CONTINUE_SEARCH
    }
    EXC_PTRS.store(ptrs as usize, Ordering::SeqCst);
    EXC_TID.store(GetCurrentThreadId(), Ordering::SeqCst);
    // Nada de alocação aqui: a pilha desta thread pode estar estourada.
    SetEvent(REQ_EVT.load(Ordering::SeqCst) as HANDLE);
    WaitForSingleObject(DONE_EVT.load(Ordering::SeqCst) as HANDLE, 120_000);
    1 // EXCEPTION_EXECUTE_HANDLER: encerra o processo
}

fn exception_name(code: u32) -> &'static str {
    match code {
        0xC000_0005 => "ACCESS_VIOLATION",
        0xC000_00FD => "STACK_OVERFLOW",
        0xC000_0094 => "INTEGER_DIVIDE_BY_ZERO",
        0xC000_001D => "ILLEGAL_INSTRUCTION",
        0x8000_0003 => "BREAKPOINT",
        0xC000_0409 => "STACK_BUFFER_OVERRUN / fail-fast",
        _ => "exceção desconhecida",
    }
}

fn handler_thread() {
    let req = REQ_EVT.load(Ordering::SeqCst) as HANDLE;
    let done = DONE_EVT.load(Ordering::SeqCst) as HANDLE;
    loop {
        unsafe { WaitForSingleObject(req, 0xFFFF_FFFF) };
        let ptrs = EXC_PTRS.load(Ordering::SeqCst);
        let tid = EXC_TID.load(Ordering::SeqCst);
        let mut details = String::new();
        unsafe {
            let ep = ptrs as *const ExceptionPointers;
            if !ep.is_null() && !(*ep).record.is_null() {
                let rec = &*(*ep).record;
                let _ = writeln!(details, "exceção: 0x{:08X} {} no endereço {:p}", rec.code, exception_name(rec.code), rec.address);
                if rec.code == 0xC000_0005 && rec.nparams >= 2 {
                    let op = match rec.info[0] {
                        0 => "leitura",
                        1 => "escrita",
                        8 => "execução",
                        _ => "acesso",
                    };
                    let _ = writeln!(details, "operação: {} em 0x{:X}", op, rec.info[1]);
                }
            }
        }
        let lane = (0..MAX_LANES).find(|&l| LANE_TID[l].load(Ordering::Relaxed) == tid);
        match lane {
            Some(l) => {
                let _ = writeln!(
                    details,
                    "thread que falhou: {} (tid {}), executando {}",
                    lane_name(l),
                    tid,
                    sys_label(LANE_SYS[l].load(Ordering::Relaxed))
                );
            }
            None => {
                let _ = writeln!(details, "thread que falhou: tid {} (fora do job system)", tid);
            }
        }
        let _ = writeln!(details, "\nA pilha completa da thread está no dump.dmp (abra com os .pdb).");
        let title = lane
            .map(|l| format!("crash nativo em {}", sys_label(LANE_SYS[l].load(Ordering::Relaxed))))
            .unwrap_or_else(|| "crash nativo".into());
        if let Some(dir) = write_report("crash", &title, &details, false) {
            write_dump(&dir.join("dump.dmp"), Some((tid, ptrs)));
            if CONFIG.get().is_some_and(|c| c.interactive) {
                platform::message_box("forja: crash nativo", &format!("{}\n\n{}\nRelatório, minidump e replay em:\n{}", title, details, dir.display()));
            }
        }
        unsafe { SetEvent(done) };
    }
}

// ---------------------------------------------------------------- watchdog

fn watchdog_thread() {
    const LIMIT: Duration = Duration::from_secs(4);
    let mut last = HEARTBEAT.load(Ordering::Relaxed);
    let mut since = Instant::now();
    let mut hung_at: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let hb = HEARTBEAT.load(Ordering::Relaxed);
        if WATCHDOG_PAUSED.load(Ordering::Relaxed) || hb == 0 {
            last = hb;
            since = Instant::now();
            continue;
        }
        if hb != last {
            if let Some(t) = hung_at.take() {
                breadcrumb(format!("travamento terminou depois de {:.1} s", t.elapsed().as_secs_f32() + LIMIT.as_secs_f32()));
            }
            last = hb;
            since = Instant::now();
        } else if hung_at.is_none() && since.elapsed() > LIMIT {
            hung_at = Some(Instant::now());
            breadcrumb("watchdog: frame parado há 4 s");
            write_report(
                "hang",
                "frame não avança há 4 s",
                "O watchdog não viu o heartbeat do frame avançar.\nVeja acima o que cada lane executava: a lane presa aponta o sistema.\nO dump.dmp tem a pilha de todas as threads.",
                true,
            );
        }
    }
}
