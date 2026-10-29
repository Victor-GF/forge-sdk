//! Escrita de arquivo via Win32, sem alocar (o caminho vira UTF-16 na stack).
//! A std (`std::fs`) aloca para converter caminhos no Windows, por isso não é usada.

use core::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile,
};

use crate::utf16_into;

/// Tamanho máximo de caminho (em unidades UTF-16, com o 0 final).
pub const PATH_MAX: usize = 260;

/// Arquivo aberto para escrita. Só existe aberto: `create` devolve `None` se falhar e
/// `close` consome o valor.
pub struct FileWriter {
    handle: HANDLE,
}

impl FileWriter {
    /// Cria (ou sobrescreve) o arquivo. `None` se o caminho for longo demais ou o SO recusar.
    pub fn create(path: &str) -> Option<FileWriter> {
        let mut buf = [0u16; PATH_MAX];
        let p = utf16_into(path, &mut buf)?;
        // SAFETY: `p` termina em 0 e vive durante a chamada; sem atributos de segurança nem template.
        let handle = unsafe {
            CreateFileW(p.as_ptr(), GENERIC_WRITE, 0, null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut())
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        Some(FileWriter { handle })
    }

    /// Escreve tudo. `false` se o SO falhar no meio.
    pub fn write_all(&self, data: &[u8]) -> bool {
        let mut rest = data;
        while !rest.is_empty() {
            let n = u32::try_from(rest.len()).unwrap_or(u32::MAX);
            let mut written: u32 = 0;
            // SAFETY: `rest` tem pelo menos `n` bytes; `written` é local; escrita síncrona.
            let ok = unsafe { WriteFile(self.handle, rest.as_ptr(), n, &mut written, null_mut()) };
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

    pub fn close(self) {
        // SAFETY: `handle` veio de `create` e é fechado uma vez só (`self` é consumido).
        unsafe { CloseHandle(self.handle) };
    }
}
