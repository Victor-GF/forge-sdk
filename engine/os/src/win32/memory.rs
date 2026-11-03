use super::{Win32, ffi};
use crate::memory::{Memory, PageInfo};
use core::{mem, ptr};

impl Memory for Win32 {
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
    // Sem `HEAP_GENERATE_EXCEPTIONS`, `HeapAlloc`/`HeapReAlloc` devolvem
    // NULL ao falhar em vez de levantar excecao.

    #[inline]
    fn heap_alloc(size: usize) -> *mut u8 {
        let heap = process_heap();
        if heap.is_null() {
            return ptr::null_mut();
        }
        unsafe { ffi::HeapAlloc(heap, 0, size) as *mut u8 }
    }

    #[inline]
    fn heap_alloc_zeroed(size: usize) -> *mut u8 {
        let heap = process_heap();
        if heap.is_null() {
            return ptr::null_mut();
        }
        unsafe { ffi::HeapAlloc(heap, ffi::HEAP_ZERO_MEMORY, size) as *mut u8 }
    }

    #[inline]
    unsafe fn heap_realloc(ptr: *mut u8, size: usize) -> *mut u8 {
        let heap = process_heap();
        if heap.is_null() {
            return ptr::null_mut();
        }
        unsafe { ffi::HeapReAlloc(heap, 0, ptr.cast(), size) as *mut u8 }
    }

    /// Sem heap do processo nenhum bloco pode ter sido alocado, entao nao ha
    /// o que liberar.
    #[inline]
    unsafe fn heap_free(ptr: *mut u8) {
        let heap = process_heap();
        if ptr.is_null() || heap.is_null() {
            return;
        }
        unsafe {
            ffi::HeapFree(heap, 0, ptr.cast());
        }
    }
}

/// Handle do heap do processo, nulo se `GetProcessHeap` falhar.
///
/// A documentacao preve NULL, e passar NULL adiante derruba o processo dentro
/// de `HeapAlloc` em vez de devolver falha. Na pratica a chamada so le
/// `PEB->ProcessHeap`, criado pelo loader antes de qualquer codigo nosso;
/// por isso tambem nao vale guardar o handle em estado global.
#[inline]
fn process_heap() -> ffi::HANDLE {
    unsafe { ffi::GetProcessHeap() }
}
