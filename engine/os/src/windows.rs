//! Backend Windows do `forja_os` (windows-sys). Ver o contrato em `lib.rs`.

use core::ffi::c_void;
use core::fmt::Write;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicPtr, Ordering};

use forja_core::text::TextBuf;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FreeLibrary, GENERIC_WRITE, GetLastError, HANDLE, HMODULE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    EXCEPTION_POINTERS, RtlCaptureStackBackTrace, SetUnhandledExceptionFilter,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW, GetProcAddress, LoadLibraryW,
};
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::SystemInformation::{
    GetSystemInfo, GetSystemTime, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, SetThreadStackGuarantee, Sleep, TerminateProcess,
};

use crate::fatal::{FatalEvent, FatalKind, MemoryAccess};
use crate::file::PATH_MAX;
use crate::library::NAME_MAX;
use crate::memory::PageInfo;
use crate::process::{ProcessMemory, SystemMemory};
use crate::stack::FRAMES_MAX;
use crate::time::UtcTime;

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

// ---------------------------------------------------------------- memória

pub(crate) fn page_info() -> PageInfo {
    // SAFETY: SYSTEM_INFO só tem inteiros e ponteiros; zero é válido; a função só escreve.
    let info: SYSTEM_INFO = unsafe {
        let mut i = core::mem::zeroed();
        GetSystemInfo(&mut i);
        i
    };
    PageInfo {
        page_size: info.dwPageSize as usize,
        allocation_granularity: info.dwAllocationGranularity as usize,
    }
}

pub(crate) fn memory_reserve(size: usize) -> Option<*mut u8> {
    // SAFETY: pedido simples de memória nova; o resultado é checado.
    let p = unsafe { VirtualAlloc(null(), size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
    if p.is_null() { None } else { Some(p.cast::<u8>()) }
}

// ---------------------------------------------------------------- tempo

pub(crate) fn ticks() -> u64 {
    let mut v: i64 = 0;
    // SAFETY: ponteiro para variável local válida.
    unsafe { QueryPerformanceCounter(&mut v) };
    u64::try_from(v).unwrap_or(0)
}

pub(crate) fn ticks_per_second() -> u64 {
    let mut v: i64 = 0;
    // SAFETY: ponteiro para variável local válida.
    unsafe { QueryPerformanceFrequency(&mut v) };
    u64::try_from(v).unwrap_or(1)
}

pub(crate) fn sleep_ms(ms: u32) {
    // SAFETY: chamada sem ponteiros.
    unsafe { Sleep(ms) };
}

pub(crate) fn begin_precise_sleep() {
    // SAFETY: chamada sem ponteiros.
    unsafe { timeBeginPeriod(1) };
}

pub(crate) fn end_precise_sleep() {
    // SAFETY: chamada sem ponteiros.
    unsafe { timeEndPeriod(1) };
}

pub(crate) fn utc_now() -> UtcTime {
    // SAFETY: SYSTEMTIME só tem inteiros; zero é válido; a função só escreve.
    let st = unsafe {
        let mut st = core::mem::zeroed();
        GetSystemTime(&mut st);
        st
    };
    UtcTime {
        year: st.wYear,
        month: u8::try_from(st.wMonth).unwrap_or(0),
        day: u8::try_from(st.wDay).unwrap_or(0),
        hour: u8::try_from(st.wHour).unwrap_or(0),
        minute: u8::try_from(st.wMinute).unwrap_or(0),
        second: u8::try_from(st.wSecond).unwrap_or(0),
    }
}

// ---------------------------------------------------------------- arquivos

pub(crate) type FileHandle = HANDLE;

pub(crate) fn file_create(path: &str) -> Option<FileHandle> {
    let mut buf = [0u16; PATH_MAX + 1];
    let p = utf16_into(path, &mut buf)?;
    // SAFETY: `p` termina em 0 e vive durante a chamada; sem atributos de segurança nem template.
    let h = unsafe {
        CreateFileW(p.as_ptr(), GENERIC_WRITE, 0, null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    if h == INVALID_HANDLE_VALUE { None } else { Some(h) }
}

pub(crate) fn file_write_all(h: &FileHandle, data: &[u8]) -> bool {
    let mut rest = data;
    while !rest.is_empty() {
        let n = u32::try_from(rest.len()).unwrap_or(u32::MAX);
        let mut written: u32 = 0;
        // SAFETY: `rest` tem pelo menos `n` bytes; `written` é local; escrita síncrona.
        let ok = unsafe { WriteFile(*h, rest.as_ptr(), n, &mut written, null_mut()) };
        if ok == 0 || written == 0 {
            return false;
        }
        let Some(r) = rest.get(written as usize..) else {
            return false;
        };
        rest = r;
    }
    true
}

pub(crate) fn file_close(h: FileHandle) {
    // SAFETY: `h` veio de `file_create` e é fechado uma vez (o `FileWriter` foi consumido).
    unsafe { CloseHandle(h) };
}

pub(crate) fn create_dir(path: &str) -> bool {
    let mut buf = [0u16; PATH_MAX + 1];
    let Some(p) = utf16_into(path, &mut buf) else {
        return false;
    };
    // SAFETY: `p` termina em 0; sem atributos de segurança. `GetLastError` logo em seguida.
    unsafe { CreateDirectoryW(p.as_ptr(), null()) != 0 || GetLastError() == ERROR_ALREADY_EXISTS }
}

// ---------------------------------------------------------------- pilha

/// Abaixo disso o Windows nunca mapeia código: endereços assim no fim da pilha são lixo do fim
/// da cadeia de quadros.
const LOWEST_CODE_ADDRESS: usize = 0x1_0000;

#[inline(always)]
pub(crate) fn stack_capture(skip: u32, frames: &mut [usize]) -> usize {
    let mut raw = [null_mut::<c_void>(); FRAMES_MAX];
    let wanted = u32::try_from(frames.len().min(FRAMES_MAX)).unwrap_or(0);
    // SAFETY: `raw` tem espaço para `FRAMES_MAX` ponteiros e `wanted <= FRAMES_MAX`.
    let n = unsafe { RtlCaptureStackBackTrace(skip, wanted, raw.as_mut_ptr(), null_mut()) };
    let mut count: usize = 0;
    for (dst, src) in frames.iter_mut().zip(raw.iter()).take(usize::from(n)) {
        let address = *src as usize;
        if address < LOWEST_CODE_ADDRESS {
            break;
        }
        *dst = address;
        count = count.saturating_add(1);
    }
    count
}

pub(crate) fn write_module_offset(t: &mut TextBuf, address: usize) {
    let mut module: HMODULE = null_mut();
    // SAFETY: com FROM_ADDRESS o "nome" é um endereço qualquer; `module` é local.
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    if ok == 0 || module.is_null() {
        let _ = write!(t, "0x{address:x}");
        return;
    }
    let mut name = [0u16; 260];
    // SAFETY: `name` tem 260 posições.
    let n = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), 260) };
    let path = name.get(..n as usize).unwrap_or(&[]);
    let start = path.iter().rposition(|&c| c == u16::from(b'\\')).map_or(0, |i| i.saturating_add(1));
    let file = path.get(start..).unwrap_or(&[]);
    for c in char::decode_utf16(file.iter().copied()) {
        let _ = t.write_char(c.unwrap_or('?'));
    }
    let _ = write!(t, "+0x{:x}", address.wrapping_sub(module as usize));
}

// ---------------------------------------------------------------- erro fatal

const EXCEPTION_EXECUTE_HANDLER: i32 = 1;
/// Reserva de stack garantida ao handler na thread que chamou `install`.
const HANDLER_STACK_GUARANTEE: u32 = 64 * 1024;

static HANDLER: AtomicPtr<()> = AtomicPtr::new(null_mut());

pub(crate) fn fatal_install(handler: fn(&FatalEvent)) {
    HANDLER.store(handler as *mut (), Ordering::Release);
    let mut guarantee = HANDLER_STACK_GUARANTEE;
    // SAFETY: `guarantee` é local; o filtro é uma função estática.
    unsafe {
        SetThreadStackGuarantee(&mut guarantee);
        SetUnhandledExceptionFilter(Some(exception_filter));
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

fn kind_from_exception(code: u32, p0: usize, p1: usize) -> FatalKind {
    match code {
        0xC000_0005 => FatalKind::AccessViolation {
            address: p1,
            access: match p0 {
                0 => MemoryAccess::Read,
                1 => MemoryAccess::Write,
                8 => MemoryAccess::Execute,
                _ => MemoryAccess::Unknown,
            },
        },
        0xC000_00FD => FatalKind::StackOverflow,
        0xC000_001D | 0xC000_0096 => FatalKind::IllegalInstruction,
        0xC000_0094 => FatalKind::DivideByZero,
        0xC000_0095 => FatalKind::IntegerOverflow,
        0xC000_0006 => FatalKind::BusError,
        0x8000_0003 => FatalKind::Breakpoint,
        _ => FatalKind::Other,
    }
}

unsafe extern "system" fn exception_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: o SO passa ponteiros válidos (checados contra nulo).
    let record = unsafe { info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) };
    let event = match record {
        Some(r) => {
            let code = r.ExceptionCode as u32;
            let p0 = r.ExceptionInformation.first().copied().unwrap_or(0);
            let p1 = r.ExceptionInformation.get(1).copied().unwrap_or(0);
            FatalEvent {
                kind: kind_from_exception(code, p0, p1),
                os_code: code,
                instruction_address: r.ExceptionAddress as usize,
            }
        }
        None => FatalEvent { kind: FatalKind::Other, os_code: 0, instruction_address: 0 },
    };
    if let Some(handler) = installed_handler() {
        handler(&event);
    }
    EXCEPTION_EXECUTE_HANDLER
}

pub(crate) fn terminate(exit_code: u32) -> ! {
    // SAFETY: encerra o próprio processo.
    unsafe { TerminateProcess(GetCurrentProcess(), exit_code) };
    loop {
        // TerminateProcess no próprio processo não volta; o loop só satisfaz o tipo `!`.
        sleep_ms(1000);
    }
}

// ---------------------------------------------------------------- bibliotecas dinâmicas

pub(crate) type LibraryHandle = HMODULE;

pub(crate) fn library_open(name: &str) -> Option<LibraryHandle> {
    let mut buf = [0u16; NAME_MAX + 1];
    let n = utf16_into(name, &mut buf)?;
    // SAFETY: `n` termina em 0 e vive durante a chamada.
    let h = unsafe { LoadLibraryW(n.as_ptr()) };
    if h.is_null() { None } else { Some(h) }
}

pub(crate) fn library_symbol(h: &LibraryHandle, name: &str) -> Option<*mut c_void> {
    // GetProcAddress recebe o nome em ANSI terminado em 0.
    let mut buf = [0u8; NAME_MAX + 1];
    let len = name.len();
    buf.get_mut(..len)?.copy_from_slice(name.as_bytes());
    *buf.get_mut(len)? = 0;
    // SAFETY: `h` veio de LoadLibraryW; `buf` termina em 0.
    let f = unsafe { GetProcAddress(*h, buf.as_ptr()) };
    f.map(|f| f as *const () as *mut c_void)
}

pub(crate) fn library_close(h: LibraryHandle) {
    // SAFETY: `h` veio de LoadLibraryW e é liberado uma vez (a `Library` foi consumida).
    unsafe { FreeLibrary(h) };
}

// ---------------------------------------------------------------- processo

pub(crate) fn thread_id() -> u64 {
    // SAFETY: chamada sem ponteiros.
    u64::from(unsafe { GetCurrentThreadId() })
}

pub(crate) fn system_memory() -> Option<SystemMemory> {
    // SAFETY: struct C só com inteiros (zero é válido); tamanho preenchido antes da chamada.
    let m = unsafe {
        let mut m: MEMORYSTATUSEX = core::mem::zeroed();
        m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) == 0 {
            return None;
        }
        m
    };
    Some(SystemMemory {
        total_physical: m.ullTotalPhys,
        available_physical: m.ullAvailPhys,
        commit_limit: Some(m.ullTotalPageFile),
        commit_available: Some(m.ullAvailPageFile),
    })
}

pub(crate) fn process_memory() -> Option<ProcessMemory> {
    // SAFETY: struct C só com inteiros (zero é válido); tamanho preenchido antes da chamada.
    let p = unsafe {
        let mut p: PROCESS_MEMORY_COUNTERS = core::mem::zeroed();
        p.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb) == 0 {
            return None;
        }
        p
    };
    Some(ProcessMemory {
        resident: p.WorkingSetSize as u64,
        peak_resident: p.PeakWorkingSetSize as u64,
        committed: Some(p.PagefileUsage as u64),
        peak_committed: Some(p.PeakPagefileUsage as u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_is_null_terminated_and_respects_buffer() {
        let mut buf = [0xFFFFu16; 4];
        assert_eq!(utf16_into("ab", &mut buf), Some(&[0x61, 0x62, 0][..]));
        assert_eq!(utf16_into("abcd", &mut buf), None); // falta espaço para o 0
    }

    #[test]
    fn access_violation_decodes_access_kind() {
        assert_eq!(
            kind_from_exception(0xC000_0005, 1, 0x10),
            FatalKind::AccessViolation { address: 0x10, access: MemoryAccess::Write }
        );
        assert_eq!(kind_from_exception(0xC000_00FD, 0, 0), FatalKind::StackOverflow);
        assert_eq!(kind_from_exception(0x1234, 0, 0), FatalKind::Other);
    }
}
