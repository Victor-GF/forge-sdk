use super::{Win32, ffi, wide};
use crate::diagnostics::Diagnostics;
use core::ptr;

impl Diagnostics for Win32 {
    /// Console de verdade recebe UTF-16 por `WriteConsoleW`, que mostra
    /// acentos certos qualquer que seja o code page. Saida redirecionada
    /// (arquivo, pipe) recebe os bytes UTF-8 como estao.
    fn console_write(text: &str) {
        if text.is_empty() {
            return;
        }
        unsafe {
            let out = ffi::GetStdHandle(ffi::STD_OUTPUT_HANDLE);
            if out.is_null() || out == ffi::INVALID_HANDLE_VALUE {
                return;
            }
            let mut mode: ffi::DWORD = 0;
            if ffi::GetConsoleMode(out, &mut mode) != 0 {
                wide::for_each_chunk(text, |chunk| {
                    ffi::WriteConsoleW(
                        out,
                        chunk.as_ptr().cast(),
                        (chunk.len() - 1) as ffi::DWORD,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                });
            } else {
                write_all(out, text.as_bytes());
            }
        }
    }

    fn debug_write(text: &str) {
        wide::for_each_chunk(text, |chunk| unsafe { ffi::OutputDebugStringW(chunk.as_ptr()) });
    }

    #[inline]
    fn debugger_present() -> bool {
        unsafe { ffi::IsDebuggerPresent() != 0 }
    }

    #[inline]
    fn debug_break() {
        unsafe { ffi::DebugBreak() }
    }
}

/// `WriteFile` pode escrever menos que o pedido num pipe: repete ate acabar.
/// Falha descarta o resto; log nao tem a quem reportar.
unsafe fn write_all(file: ffi::HANDLE, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let size = bytes.len().min(u32::MAX as usize) as ffi::DWORD;
        let mut written: ffi::DWORD = 0;
        let ok = unsafe { ffi::WriteFile(file, bytes.as_ptr().cast(), size, &mut written, ptr::null_mut()) };
        if ok == 0 || written == 0 {
            return;
        }
        bytes = &bytes[written as usize..];
    }
}
