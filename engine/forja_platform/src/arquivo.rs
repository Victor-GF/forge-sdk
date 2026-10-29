//! Escrita de arquivo via Win32, sem alocar (o caminho vira UTF-16 na stack).
//! A std (`std::fs`) aloca para converter caminhos no Windows, por isso não é usada.

use core::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile,
};

use crate::utf16_em;

/// Tamanho máximo de caminho (em unidades UTF-16, com o 0 final).
pub const CAMINHO_MAX: usize = 260;

pub struct Arquivo {
    handle: HANDLE,
}

/// Cria (ou sobrescreve) o arquivo para escrita. `None` se o caminho for longo demais ou o
/// SO recusar.
pub fn arquivo_criar(caminho: &str) -> Option<Arquivo> {
    let mut buf = [0u16; CAMINHO_MAX];
    let c = utf16_em(caminho, &mut buf)?;
    // SAFETY: `c` termina em 0 e vive durante a chamada; sem atributos de segurança nem template.
    let handle = unsafe {
        CreateFileW(c.as_ptr(), GENERIC_WRITE, 0, null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    Some(Arquivo { handle })
}

/// Escreve tudo. `false` se o SO falhar no meio.
pub fn arquivo_escrever(a: &Arquivo, dados: &[u8]) -> bool {
    let mut resto = dados;
    while !resto.is_empty() {
        let n = u32::try_from(resto.len()).unwrap_or(u32::MAX);
        let mut escrito: u32 = 0;
        // SAFETY: `resto` tem pelo menos `n` bytes; `escrito` é local; escrita síncrona.
        let ok = unsafe { WriteFile(a.handle, resto.as_ptr(), n, &mut escrito, null_mut()) };
        if ok == 0 || escrito == 0 {
            return false;
        }
        let Some(r) = resto.get(escrito as usize..) else {
            return false;
        };
        resto = r;
    }
    true
}

pub fn arquivo_fechar(a: Arquivo) {
    // SAFETY: `handle` veio de `arquivo_criar` e é fechado uma vez só (`a` é consumido).
    unsafe { CloseHandle(a.handle) };
}
