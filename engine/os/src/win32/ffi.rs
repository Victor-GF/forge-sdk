//! Bindings Win32 escritos a mao.
//!
//! Apenas o que o backend usa, declarado direto contra as DLLs do sistema.
//! Sem `windows-sys`, sem `winapi`: cada simbolo aqui e uma decisao explicita.
//!
//! Regras deste arquivo:
//! - Linkagem `raw-dylib`: o rustc gera a import table sozinho, sem depender
//!   das import libs (.a/.lib) do toolchain.
//! - Toda struct `#[repr(C)]` tem seu tamanho e alinhamento travados com
//!   `layout!`, usando os valores x64 do SDK.
//! - Cada bloco cita o header do Windows SDK de onde veio.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use core::ffi::{c_char, c_void};
use forge_core::layout;

// ---- minwindef.h / winnt.h / basetsd.h -------------------------------------

pub type BOOL = i32;
pub type WORD = u16;
pub type DWORD = u32;
pub type UINT = u32;
pub type INT = i32;
pub type SIZE_T = usize;
pub type DWORD_PTR = usize;
pub type LPVOID = *mut c_void;
pub type LPCSTR = *const c_char;
pub type HANDLE = *mut c_void;
pub type HWND = *mut c_void;

layout!(HANDLE, size = 8, align = 8);
layout!(HWND, size = 8, align = 8);

// ---- winnt.h: memoria ------------------------------------------------------

pub const MEM_COMMIT: DWORD = 0x0000_1000;
pub const MEM_RESERVE: DWORD = 0x0000_2000;
pub const MEM_DECOMMIT: DWORD = 0x0000_4000;
pub const MEM_RELEASE: DWORD = 0x0000_8000;

pub const PAGE_NOACCESS: DWORD = 0x01;
pub const PAGE_READWRITE: DWORD = 0x04;

pub const HEAP_ZERO_MEMORY: DWORD = 0x0000_0008;

// ---- sysinfoapi.h ----------------------------------------------------------

/// O primeiro campo no SDK e uma union `dwOemId` / `{ wProcessorArchitecture,
/// wReserved }`; aqui fica so a forma de struct, com o mesmo layout.
#[repr(C)]
pub struct SYSTEM_INFO {
    pub wProcessorArchitecture: WORD,
    pub wReserved: WORD,
    pub dwPageSize: DWORD,
    pub lpMinimumApplicationAddress: LPVOID,
    pub lpMaximumApplicationAddress: LPVOID,
    pub dwActiveProcessorMask: DWORD_PTR,
    pub dwNumberOfProcessors: DWORD,
    pub dwProcessorType: DWORD,
    pub dwAllocationGranularity: DWORD,
    pub wProcessorLevel: WORD,
    pub wProcessorRevision: WORD,
}

layout!(SYSTEM_INFO, size = 48, align = 8);

// ---- winuser.h -------------------------------------------------------------

pub const MB_OK: UINT = 0x0000_0000;
pub const MB_ICONINFORMATION: UINT = 0x0000_0040;

// ---- kernel32 ----------------------------------------------------------------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    // processthreadsapi.h
    pub fn ExitProcess(uExitCode: UINT) -> !;

    // sysinfoapi.h
    pub fn GetSystemInfo(lpSystemInfo: *mut SYSTEM_INFO);

    // memoryapi.h
    pub fn VirtualAlloc(
        lpAddress: LPVOID,
        dwSize: SIZE_T,
        flAllocationType: DWORD,
        flProtect: DWORD,
    ) -> LPVOID;
    pub fn VirtualFree(lpAddress: LPVOID, dwSize: SIZE_T, dwFreeType: DWORD) -> BOOL;

    // heapapi.h
    pub fn GetProcessHeap() -> HANDLE;
    pub fn HeapAlloc(hHeap: HANDLE, dwFlags: DWORD, dwBytes: SIZE_T) -> LPVOID;
    pub fn HeapReAlloc(hHeap: HANDLE, dwFlags: DWORD, lpMem: LPVOID, dwBytes: SIZE_T) -> LPVOID;
    pub fn HeapFree(hHeap: HANDLE, dwFlags: DWORD, lpMem: LPVOID) -> BOOL;
}

// ---- user32 ------------------------------------------------------------------

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    // winuser.h
    pub fn MessageBoxA(hWnd: HWND, lpText: LPCSTR, lpCaption: LPCSTR, uType: UINT) -> INT;
}
