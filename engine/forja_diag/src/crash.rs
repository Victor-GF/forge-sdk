//! Crash report.
//!
//! Entradas:
//! - `diag_fatal(motivo, detalhe)`: erro fatal decidido pelo código (OOM de arena, alocador
//!   travado...). Grava o relatório e aborta.
//! - panic: o gancho instalado por `diag_instalar` grava e aborta antes de a std alocar o
//!   payload do unwind.
//! - exceções do SO (access violation, stack overflow, instrução ilegal...): filtro de
//!   exceção não tratada.
//!
//! O `abort()` do Rust no Windows usa *fast fail*, que pula os filtros de exceção; por isso todo
//! caminho que aborta de propósito passa por `diag_fatal` antes.
//!
//! O relatório é montado num buffer estático (`RELATORIO_CAPACIDADE`) e gravado via Win32:
//! nada é alocado, então funciona sem memória livre e com o alocador global travado.
//! Pouca stack é usada, porque no stack overflow o handler roda com ~20 KB de reserva.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::{self, Write};
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use forja_core::texto::{Texto, texto_bytes, texto_novo, texto_str};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HMODULE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    EXCEPTION_POINTERS, RtlCaptureStackBackTrace, SetUnhandledExceptionFilter,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::SystemInformation::{
    GetLocalTime, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, Sleep, TerminateProcess,
};

use crate::log::rastro_escrever;
use crate::trava::{Trava, trava_nova, trava_soltar, trava_tentar};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiagDesc {
    pub jogo: &'static str,
    pub versao: &'static str,
    /// Pasta dos relatórios (um nível, criada se não existir).
    pub pasta: &'static str,
}

const DESC_PADRAO: DiagDesc = DiagDesc { jogo: "?", versao: "?", pasta: "crash" };

/// Seção extra do relatório: o jogo e a engine registram funções que escrevem o próprio
/// estado. Rodam durante o crash: não podem alocar nem entrar em pânico.
#[derive(Clone, Copy)]
pub struct Secao {
    pub nome: &'static str,
    pub escrever: fn(&mut Texto),
}

pub const SECOES_MAX: usize = 16;
pub const RELATORIO_CAPACIDADE: usize = 64 * 1024;
const PILHA_MAX: u32 = 62;
const EXCECAO_EXECUTAR_HANDLER: i32 = 1;

#[derive(Clone, Copy)]
struct Estado {
    desc: DiagDesc,
    secoes: [Option<Secao>; SECOES_MAX],
    n_secoes: usize,
}

struct Compartilhado {
    trava: Trava,
    estado: UnsafeCell<Estado>,
}

// SAFETY: `estado` só é escrito com `trava` pega; no crash é lido mesmo sem ela (melhor esforço).
unsafe impl Sync for Compartilhado {}

static COMPARTILHADO: Compartilhado = Compartilhado {
    trava: trava_nova(),
    estado: UnsafeCell::new(Estado { desc: DESC_PADRAO, secoes: [None; SECOES_MAX], n_secoes: 0 }),
};

struct BufRelatorio(UnsafeCell<[u8; RELATORIO_CAPACIDADE]>);

// SAFETY: só a thread que venceu `crash_entrar` usa o buffer.
unsafe impl Sync for BufRelatorio {}

static RELATORIO: BufRelatorio = BufRelatorio(UnsafeCell::new([0; RELATORIO_CAPACIDADE]));
static INICIO_MS: AtomicU64 = AtomicU64::new(0);
static FRAME: AtomicU64 = AtomicU64::new(0);
/// Thread que está escrevendo o relatório (0 = nenhuma).
static CRASH_THREAD: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------- API

/// Instala o gancho de panic e o filtro de exceções. Chame no início do `main`, antes de
/// travar o alocador global.
pub fn diag_instalar(desc: DiagDesc) {
    if trava_tentar(&COMPARTILHADO.trava, u32::MAX) {
        // SAFETY: trava pega.
        unsafe { (*COMPARTILHADO.estado.get()).desc = desc };
        trava_soltar(&COMPARTILHADO.trava);
    }
    // SAFETY: chamadas sem ponteiros (o filtro é uma função estática).
    unsafe {
        INICIO_MS.store(GetTickCount64(), Ordering::Relaxed);
        SetUnhandledExceptionFilter(Some(filtro_excecao));
    }
    // `gancho_panic` é uma função (tamanho zero): o Box não aloca.
    std::panic::set_hook(Box::new(gancho_panic));
}

/// Registra uma seção extra no relatório. `false` se já houver `SECOES_MAX`.
pub fn diag_registrar_secao(nome: &'static str, escrever: fn(&mut Texto)) -> bool {
    if !trava_tentar(&COMPARTILHADO.trava, u32::MAX) {
        return false;
    }
    // SAFETY: trava pega.
    let e = unsafe { &mut *COMPARTILHADO.estado.get() };
    let ok = match e.secoes.get_mut(e.n_secoes) {
        Some(s) => {
            *s = Some(Secao { nome, escrever });
            e.n_secoes = e.n_secoes.saturating_add(1);
            true
        }
        None => false,
    };
    trava_soltar(&COMPARTILHADO.trava);
    ok
}

/// Frame atual, para o relatório. Barato: um store atômico.
pub fn diag_frame(indice: u64) {
    FRAME.store(indice, Ordering::Relaxed);
}

/// Erro fatal: grava o relatório e aborta.
pub fn diag_fatal(motivo: &str, detalhe: fmt::Arguments) -> ! {
    crash_relatorio(motivo, detalhe, None);
    std::process::abort()
}

// ---------------------------------------------------------------- entradas do SO e da std

fn gancho_panic(info: &std::panic::PanicHookInfo) {
    let msg = info.payload_as_str().unwrap_or("(mensagem não textual)");
    match info.location() {
        Some(l) => crash_relatorio(
            "panic",
            format_args!("{msg}\n         em {}:{}:{}", l.file(), l.line(), l.column()),
            None,
        ),
        None => crash_relatorio("panic", format_args!("{msg}"), None),
    }
    std::process::abort()
}

unsafe extern "system" fn filtro_excecao(info: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: o SO passa ponteiros válidos (checados contra nulo).
    let registro = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
    let Some(r) = registro else {
        crash_relatorio("exceção", format_args!("sem registro de exceção"), None);
        return EXCECAO_EXECUTAR_HANDLER;
    };
    let codigo = r.ExceptionCode as u32;
    let endereco = r.ExceptionAddress as usize;
    let p0 = r.ExceptionInformation.first().copied().unwrap_or(0);
    let p1 = r.ExceptionInformation.get(1).copied().unwrap_or(0);
    let nome = excecao_nome(codigo);
    if codigo == 0xC000_0005 {
        let acao = match p0 {
            0 => "ler",
            1 => "escrever",
            8 => "executar",
            _ => "acessar",
        };
        crash_relatorio(
            nome,
            format_args!("código 0x{codigo:08X}: tentou {acao} o endereço 0x{p1:016x}"),
            Some(endereco),
        );
    } else {
        crash_relatorio(nome, format_args!("código 0x{codigo:08X}"), Some(endereco));
    }
    EXCECAO_EXECUTAR_HANDLER
}

fn excecao_nome(codigo: u32) -> &'static str {
    match codigo {
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
fn crash_entrar() -> bool {
    // SAFETY: chamada sem ponteiros.
    let eu = unsafe { GetCurrentThreadId() };
    match CRASH_THREAD.compare_exchange(0, eu, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => true,
        Err(dono) if dono == eu => {
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

fn estado_ler() -> Estado {
    let pegou = trava_tentar(&COMPARTILHADO.trava, 1000);
    // SAFETY: com a trava, ou no crash (melhor esforço).
    let e = unsafe { *COMPARTILHADO.estado.get() };
    if pegou {
        trava_soltar(&COMPARTILHADO.trava);
    }
    e
}

fn crash_relatorio(motivo: &str, detalhe: fmt::Arguments, excecao: Option<usize>) {
    if !crash_entrar() {
        return;
    }
    let estado = estado_ler();
    // SAFETY: só esta thread chega aqui (`crash_entrar`).
    let buf: &mut [u8] = unsafe { &mut *RELATORIO.0.get() };
    let mut t = texto_novo(buf);

    // SAFETY: SYSTEMTIME só tem inteiros; zero é válido. Chamadas sem ponteiros externos.
    let (agora, ms, thread) = unsafe {
        let mut st = core::mem::zeroed();
        GetLocalTime(&mut st);
        (st, GetTickCount64(), GetCurrentThreadId())
    };
    let rodando_s = ms.saturating_sub(INICIO_MS.load(Ordering::Relaxed)) as f64 / 1000.0;

    let _ = writeln!(t, "=== Forja: relatório de crash ===");
    let _ = writeln!(t, "jogo:    {} {}", estado.desc.jogo, estado.desc.versao);
    let _ = writeln!(
        t,
        "data:    {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        agora.wYear, agora.wMonth, agora.wDay, agora.wHour, agora.wMinute, agora.wSecond
    );
    let _ = writeln!(t, "rodando: {rodando_s:.3} s");
    let _ = writeln!(t, "frame:   {}", FRAME.load(Ordering::Relaxed));
    let _ = writeln!(t, "thread:  {thread}");
    let _ = writeln!(t, "motivo:  {motivo}");
    let _ = writeln!(t, "detalhe: {detalhe}");

    escrever_pilha(&mut t, excecao);
    escrever_memoria_sistema(&mut t);
    for s in estado.secoes.iter().flatten() {
        let _ = writeln!(t, "\n--- {} ---", s.nome);
        (s.escrever)(&mut t);
    }
    let _ = writeln!(t, "\n--- rastro (últimas linhas de log) ---");
    rastro_escrever(&mut t);
    if t.truncou {
        let _ = t.write_str("\n[relatório cortado: buffer cheio]\n");
    }

    let mut caminho_buf = [0u8; 512];
    let mut caminho = texto_novo(&mut caminho_buf);
    let _ = write!(
        caminho,
        "{}/crash_{:04}-{:02}-{:02}_{:02}{:02}{:02}.txt",
        estado.desc.pasta, agora.wYear, agora.wMonth, agora.wDay, agora.wHour, agora.wMinute, agora.wSecond
    );
    criar_pasta(estado.desc.pasta);
    let gravou = arquivo_gravar(texto_str(&caminho), texto_bytes(&t));

    let mut aviso_buf = [0u8; 1024];
    let mut aviso = texto_novo(&mut aviso_buf);
    let _ = writeln!(aviso, "[crash] {motivo}: {detalhe}");
    if gravou {
        let _ = writeln!(aviso, "[crash] relatório em {}", texto_str(&caminho));
    } else {
        let _ = writeln!(aviso, "[crash] falha ao gravar o relatório em {}", texto_str(&caminho));
    }
    let _ = std::io::Write::write_all(&mut std::io::stderr(), texto_bytes(&aviso));
}

fn escrever_pilha(t: &mut Texto, excecao: Option<usize>) {
    let _ = writeln!(t, "\n--- pilha (módulo+deslocamento; simbolize com addr2line) ---");
    if let Some(endereco) = excecao {
        let _ = t.write_str("exceção ");
        escrever_endereco(t, endereco);
    }
    let mut quadros = [null_mut::<c_void>(); PILHA_MAX as usize];
    // SAFETY: `quadros` tem espaço para `PILHA_MAX` ponteiros.
    let n = unsafe { RtlCaptureStackBackTrace(1, PILHA_MAX, quadros.as_mut_ptr(), null_mut()) };
    for (i, q) in quadros.iter().take(usize::from(n)).enumerate() {
        let _ = write!(t, "#{i:02}     ");
        escrever_endereco(t, *q as usize);
    }
}

fn escrever_endereco(t: &mut Texto, endereco: usize) {
    let _ = write!(t, "0x{endereco:016x}");
    let mut modulo: HMODULE = null_mut();
    // SAFETY: com FROM_ADDRESS o "nome" é um endereço qualquer; `modulo` é local.
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            endereco as *const u16,
            &mut modulo,
        )
    };
    if ok != 0 && !modulo.is_null() {
        let mut nome = [0u16; 260];
        // SAFETY: `nome` tem 260 posições.
        let n = unsafe { GetModuleFileNameW(modulo, nome.as_mut_ptr(), 260) };
        let caminho = nome.get(..n as usize).unwrap_or(&[]);
        let inicio = caminho.iter().rposition(|&c| c == u16::from(b'\\')).map_or(0, |i| i.saturating_add(1));
        let arquivo = caminho.get(inicio..).unwrap_or(&[]);
        let _ = t.write_char(' ');
        for c in char::decode_utf16(arquivo.iter().copied()) {
            let _ = t.write_char(c.unwrap_or('?'));
        }
        let _ = write!(t, "+0x{:x}", endereco.wrapping_sub(modulo as usize));
    }
    let _ = t.write_char('\n');
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn escrever_memoria_sistema(t: &mut Texto) {
    let _ = writeln!(t, "\n--- memória do sistema ---");
    // SAFETY: structs C só com inteiros (zero é válido); tamanhos preenchidos antes da chamada.
    unsafe {
        let mut m: MEMORYSTATUSEX = core::mem::zeroed();
        m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 {
            let _ = writeln!(
                t,
                "RAM:      {:.0} MB livres de {:.0} MB (carga {}%)",
                mb(m.ullAvailPhys),
                mb(m.ullTotalPhys),
                m.dwMemoryLoad
            );
            let _ = writeln!(
                t,
                "commit:   {:.0} MB livres de {:.0} MB (limite do sistema)",
                mb(m.ullAvailPageFile),
                mb(m.ullTotalPageFile)
            );
        }
        let mut p: PROCESS_MEMORY_COUNTERS = core::mem::zeroed();
        p.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb) != 0 {
            let _ = writeln!(
                t,
                "processo: working set {:.1} MB (pico {:.1}), commit {:.1} MB (pico {:.1})",
                mb(p.WorkingSetSize as u64),
                mb(p.PeakWorkingSetSize as u64),
                mb(p.PagefileUsage as u64),
                mb(p.PeakPagefileUsage as u64)
            );
        }
    }
}

// ---------------------------------------------------------------- arquivo (Win32, sem alocar)

fn utf16_em<'b>(s: &str, buf: &'b mut [u16]) -> Option<&'b [u16]> {
    let mut n: usize = 0;
    for c in s.encode_utf16() {
        *buf.get_mut(n)? = c;
        n = n.checked_add(1)?;
    }
    *buf.get_mut(n)? = 0;
    buf.get(..=n)
}

fn criar_pasta(pasta: &str) {
    let mut buf = [0u16; 260];
    if let Some(p) = utf16_em(pasta, &mut buf) {
        // SAFETY: `p` termina em 0. Falha (pasta já existe) é ignorada.
        unsafe { CreateDirectoryW(p.as_ptr(), null()) };
    }
}

fn arquivo_gravar(caminho: &str, dados: &[u8]) -> bool {
    let mut buf = [0u16; 260];
    let Some(c) = utf16_em(caminho, &mut buf) else {
        return false;
    };
    // SAFETY: `c` termina em 0; sem atributos de segurança nem template.
    let h = unsafe {
        CreateFileW(c.as_ptr(), GENERIC_WRITE, 0, null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut resto = dados;
    let mut ok = true;
    while !resto.is_empty() {
        let n = u32::try_from(resto.len()).unwrap_or(u32::MAX);
        let mut escrito: u32 = 0;
        // SAFETY: `resto` tem pelo menos `n` bytes; escrita síncrona.
        let r = unsafe { WriteFile(h, resto.as_ptr(), n, &mut escrito, null_mut()) };
        if r == 0 || escrito == 0 {
            ok = false;
            break;
        }
        resto = resto.get(escrito as usize..).unwrap_or(&[]);
    }
    // SAFETY: `h` é válido e fechado uma vez.
    unsafe { CloseHandle(h) };
    ok
}
