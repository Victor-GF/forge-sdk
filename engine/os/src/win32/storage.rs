//! Conteudo no diretorio do executavel; saves em `%APPDATA%/<jogo>/`.
//!
//! Escrita de save atomica: grava `<nome>~tmp`, forca os dados para o disco,
//! e troca pelo definitivo com `MoveFileExW`, que substitui de uma vez e so
//! volta com a troca gravada. O sufixo usa `~`, que nao e permitido em nome
//! de save: o temporario nunca colide com um save de verdade.

use super::{Win32, ffi, wide};
use crate::file::{FileAccess, FileHandle, FileSystem, Files, PATH_CAPACITY, path_parent};
use crate::memory::Memory;
use crate::storage::{SaveHandle, Storage, save_name_valid};
use core::{mem, str};

const TEMP_SUFFIX: &str = "~tmp";

/// Caminho montado em pedacos, sem alocar.
struct PathText {
    len: usize,
    bytes: [u8; PATH_CAPACITY],
}

impl PathText {
    fn new() -> PathText {
        PathText { len: 0, bytes: [0; PATH_CAPACITY] }
    }

    /// `None` se nao couber (sobra um byte para o nulo da conversao).
    fn push(&mut self, part: &str) -> Option<()> {
        if self.len + part.len() >= PATH_CAPACITY {
            return None;
        }
        self.bytes[self.len..self.len + part.len()].copy_from_slice(part.as_bytes());
        self.len += part.len();
        Some(())
    }

    fn as_str(&self) -> &str {
        // So recebe `&str` inteiros em `push`.
        unsafe { str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }
}

/// `<area>/<nome><sufixo>`, se o nome for valido.
fn save_path(saves: &SaveHandle, name: &str, suffix: &str) -> Option<PathText> {
    if !save_name_valid(name) {
        return None;
    }
    let area = unsafe { &*(saves.0 as *const PathText) };
    let mut path = PathText::new();
    path.push(area.as_str())?;
    path.push("/")?;
    path.push(name)?;
    path.push(suffix)?;
    Some(path)
}

impl Storage for Win32 {
    fn content_open(name: &str) -> Option<FileHandle> {
        let mut exe = [0u8; PATH_CAPACITY];
        let exe = Win32::path_executable(&mut exe)?;
        let mut path = PathText::new();
        path.push(path_parent(exe))?;
        path.push("/")?;
        path.push(name)?;
        Win32::file_open(path.as_str(), FileAccess::Read)
    }

    fn save_mount(game: &str) -> Option<SaveHandle> {
        if !save_name_valid(game) {
            return None;
        }
        let mut user = [0u8; PATH_CAPACITY];
        let user = Win32::path_user_data(&mut user)?;
        let mut path = PathText::new();
        path.push(user)?;
        path.push("/")?;
        path.push(game)?;
        if !Win32::directory_create(path.as_str()) {
            return None;
        }

        let area = Win32::heap_alloc(mem::size_of::<PathText>()) as *mut PathText;
        if area.is_null() {
            return None;
        }
        unsafe { area.write(path) };
        Some(SaveHandle(area.cast()))
    }

    #[inline]
    fn save_unmount(saves: SaveHandle) {
        unsafe { Win32::heap_free(saves.0.cast()) };
    }

    fn save_write(saves: &SaveHandle, name: &str, data: &[u8]) -> bool {
        let (Some(path), Some(temp)) = (save_path(saves, name, ""), save_path(saves, name, TEMP_SUFFIX)) else {
            return false;
        };
        let Some(file) = Win32::file_open(temp.as_str(), FileAccess::Write) else {
            return false;
        };
        // Os dados precisam estar no disco antes da troca: senao, uma queda de
        // energia logo depois deixaria o save novo vazio no lugar do antigo.
        let written = Win32::file_write_at(&file, 0, data) && unsafe { ffi::FlushFileBuffers(file.0) } != 0;
        Win32::file_close(file);
        if !written {
            Win32::file_delete(temp.as_str());
            return false;
        }

        let (Some(from), Some(to)) = (
            wide::exact::<PATH_CAPACITY>(temp.as_str()),
            wide::exact::<PATH_CAPACITY>(path.as_str()),
        ) else {
            return false;
        };
        let flags = ffi::MOVEFILE_REPLACE_EXISTING | ffi::MOVEFILE_WRITE_THROUGH;
        unsafe { ffi::MoveFileExW(from.as_ptr(), to.as_ptr(), flags) != 0 }
    }

    fn save_size(saves: &SaveHandle, name: &str) -> Option<u64> {
        let path = save_path(saves, name, "")?;
        let file = Win32::file_open(path.as_str(), FileAccess::Read)?;
        let size = Win32::file_size(&file);
        Win32::file_close(file);
        size
    }

    fn save_read(saves: &SaveHandle, name: &str, buf: &mut [u8]) -> Option<usize> {
        let path = save_path(saves, name, "")?;
        let file = Win32::file_open(path.as_str(), FileAccess::Read)?;
        let read = Win32::file_read_at(&file, 0, buf);
        Win32::file_close(file);
        read
    }

    fn save_delete(saves: &SaveHandle, name: &str) -> bool {
        save_path(saves, name, "").is_some_and(|path| Win32::file_delete(path.as_str()))
    }
}
