//! Backend Win32: implementa o contrato `Os` sobre as chamadas de `ffi`.

#[cfg(not(target_arch = "x86_64"))]
compile_error!("backend win32 so suporta x86_64: os layouts de `ffi` sao os do Win64.");

mod ffi;

use crate::{Os, PageInfo};
use core::ffi::CStr;
use core::{mem, ptr};

pub struct Win32;

impl Os for Win32 {
    // ---- Processo ---------------------------------------------------------

    #[inline]
    fn message_box(caption: &CStr, text: &CStr) {
        unsafe {
            ffi::MessageBoxA(
                ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                ffi::MB_OK | ffi::MB_ICONINFORMATION,
            );
        }
    }

    #[inline]
    fn exit_process(code: u32) -> ! {
        unsafe { ffi::ExitProcess(code) }
    }

    // ---- Paginas ----------------------------------------------------------

    fn page_info() -> PageInfo {
        unsafe {
            let mut info: ffi::SYSTEM_INFO = mem::zeroed();
            ffi::GetSystemInfo(&mut info);
            PageInfo {
                size: info.dwPageSize as usize,
                granularity: info.dwAllocationGranularity as usize,
            }
        }
    }

    #[inline]
    fn pages_reserve(size: usize) -> *mut u8 {
        unsafe { ffi::VirtualAlloc(ptr::null_mut(), size, ffi::MEM_RESERVE, ffi::PAGE_NOACCESS) as *mut u8 }
    }

    #[inline]
    unsafe fn pages_commit(ptr: *mut u8, size: usize) -> bool {
        unsafe { !ffi::VirtualAlloc(ptr.cast(), size, ffi::MEM_COMMIT, ffi::PAGE_READWRITE).is_null() }
    }

    #[inline]
    fn pages_alloc(size: usize) -> *mut u8 {
        unsafe {
            ffi::VirtualAlloc(
                ptr::null_mut(),
                size,
                ffi::MEM_RESERVE | ffi::MEM_COMMIT,
                ffi::PAGE_READWRITE,
            ) as *mut u8
        }
    }

    #[inline]
    unsafe fn pages_decommit(ptr: *mut u8, size: usize) {
        unsafe {
            ffi::VirtualFree(ptr.cast(), size, ffi::MEM_DECOMMIT);
        }
    }

    /// `MEM_RELEASE` exige tamanho 0: o SO libera a reserva inteira.
    #[inline]
    unsafe fn pages_release(ptr: *mut u8) {
        unsafe {
            ffi::VirtualFree(ptr.cast(), 0, ffi::MEM_RELEASE);
        }
    }

    // ---- Heap -------------------------------------------------------------
    //
    // `GetProcessHeap` so le um campo do PEB; chamar a cada vez e mais
    // simples que guardar o handle em estado global.

    #[inline]
    fn heap_alloc(size: usize) -> *mut u8 {
        unsafe { ffi::HeapAlloc(ffi::GetProcessHeap(), 0, size) as *mut u8 }
    }

    #[inline]
    fn heap_alloc_zeroed(size: usize) -> *mut u8 {
        unsafe { ffi::HeapAlloc(ffi::GetProcessHeap(), ffi::HEAP_ZERO_MEMORY, size) as *mut u8 }
    }

    #[inline]
    unsafe fn heap_realloc(ptr: *mut u8, size: usize) -> *mut u8 {
        unsafe { ffi::HeapReAlloc(ffi::GetProcessHeap(), 0, ptr.cast(), size) as *mut u8 }
    }

    #[inline]
    unsafe fn heap_free(ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        unsafe {
            ffi::HeapFree(ffi::GetProcessHeap(), 0, ptr.cast());
        }
    }
}
