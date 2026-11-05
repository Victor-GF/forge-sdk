use super::{Win32, ffi, wide};
use crate::file::PATH_CAPACITY;
use crate::library::{Library, LibraryHandle};
use core::ffi::{CStr, c_void};

impl Library for Win32 {
    /// `LoadLibraryW` pede `\` no caminho: os `/` do contrato sao trocados.
    fn library_open(path: &str) -> Option<LibraryHandle> {
        let mut path = wide::exact::<PATH_CAPACITY>(path)?;
        for unit in path.iter_mut() {
            if *unit == b'/' as u16 {
                *unit = b'\\' as u16;
            }
        }
        let module = unsafe { ffi::LoadLibraryW(path.as_ptr()) };
        if module.is_null() {
            return None;
        }
        Some(LibraryHandle(module))
    }

    #[inline]
    fn library_symbol(library: &LibraryHandle, name: &CStr) -> *mut c_void {
        unsafe { ffi::GetProcAddress(library.0, name.as_ptr()) }
    }

    #[inline]
    unsafe fn library_close(library: LibraryHandle) {
        unsafe {
            ffi::FreeLibrary(library.0);
        }
    }
}
