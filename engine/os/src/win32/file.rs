use super::{Win32, ffi, wide};
use crate::file::{EntryKind, FileAccess, FileHandle, FileSystem, FileView, Files, PATH_CAPACITY};
use core::{mem, ptr, slice, str};

/// Unidades UTF-16 de um caminho de entrada, contando o nulo. Cada byte
/// UTF-8 vira no maximo uma unidade: qualquer caminho de ate
/// `PATH_CAPACITY - 1` bytes cabe.
const PATH_UNITS: usize = PATH_CAPACITY;

/// Maior bloco por chamada de `ReadFile`/`WriteFile`, que recebem o tamanho
/// em `DWORD`.
const IO_CHUNK: usize = 1 << 30;

/// Abrir nao trava o arquivo para ninguem, como no Linux.
const SHARE_ALL: ffi::DWORD = ffi::FILE_SHARE_READ | ffi::FILE_SHARE_WRITE | ffi::FILE_SHARE_DELETE;

const DOT: u16 = b'.' as u16;

#[inline]
fn wide_path(path: &str) -> Option<[u16; PATH_UNITS]> {
    wide::exact::<PATH_UNITS>(path)
}

/// `OVERLAPPED` so com a posicao: num handle sincrono, e assim que
/// `ReadFile`/`WriteFile` recebem o offset, sem mexer em cursor compartilhado.
fn at(offset: u64) -> ffi::OVERLAPPED {
    ffi::OVERLAPPED {
        Internal: 0,
        InternalHigh: 0,
        Offset: offset as u32,
        OffsetHigh: (offset >> 32) as u32,
        hEvent: ptr::null_mut(),
    }
}

/// Caminho do SO em `out`, em UTF-8 e com `/`.
fn path_out<'a>(units: &[u16], out: &'a mut [u8]) -> Option<&'a str> {
    let len = wide::decode(units, out)?;
    let path = &mut out[..len];
    for byte in path.iter_mut() {
        if *byte == b'\\' {
            *byte = b'/';
        }
    }
    // `decode` so escreve caracteres inteiros, e trocar um byte ASCII por
    // outro mantem o UTF-8 valido.
    Some(unsafe { str::from_utf8_unchecked(path) })
}

/// `path` termina em nulo.
fn is_directory(path: &[u16]) -> bool {
    let attributes = unsafe { ffi::GetFileAttributesW(path.as_ptr()) };
    attributes != ffi::INVALID_FILE_ATTRIBUTES && attributes & ffi::FILE_ATTRIBUTE_DIRECTORY != 0
}

/// Ate o primeiro nulo.
fn until_nul(units: &[u16]) -> &[u16] {
    let len = units.iter().position(|&unit| unit == 0).unwrap_or(units.len());
    &units[..len]
}

impl Files for Win32 {
    #[inline]
    fn file_close(file: FileHandle) {
        unsafe {
            ffi::CloseHandle(file.0);
        }
    }

    fn file_size(file: &FileHandle) -> Option<u64> {
        let mut size = ffi::LARGE_INTEGER { QuadPart: 0 };
        if unsafe { ffi::GetFileSizeEx(file.0, &mut size) } == 0 {
            return None;
        }
        Some(size.QuadPart as u64)
    }

    /// Com offset, ler a partir do fim pode voltar como falha com
    /// `ERROR_HANDLE_EOF` em vez de zero bytes: e tratado como fim.
    fn file_read_at(file: &FileHandle, offset: u64, buf: &mut [u8]) -> Option<usize> {
        let mut filled = 0;
        while filled < buf.len() {
            let size = (buf.len() - filled).min(IO_CHUNK) as ffi::DWORD;
            let mut overlapped = at(offset + filled as u64);
            let mut read: ffi::DWORD = 0;
            let ok = unsafe {
                ffi::ReadFile(file.0, buf[filled..].as_mut_ptr().cast(), size, &mut read, &mut overlapped)
            };
            if ok == 0 {
                if unsafe { ffi::GetLastError() } == ffi::ERROR_HANDLE_EOF {
                    break;
                }
                return None;
            }
            if read == 0 {
                break;
            }
            filled += read as usize;
        }
        Some(filled)
    }

    /// O tamanho vai explicito no mapeamento: se o arquivo encolher entre
    /// `file_size` e aqui, o mapeamento falha em vez de ficar menor que `len`.
    fn file_map(file: &FileHandle) -> Option<FileView> {
        let size = Win32::file_size(file)?;
        if size == 0 {
            return None;
        }
        let len = usize::try_from(size).ok()?;
        unsafe {
            let mapping = ffi::CreateFileMappingW(
                file.0,
                ptr::null_mut(),
                ffi::PAGE_READONLY,
                (size >> 32) as ffi::DWORD,
                size as ffi::DWORD,
                ptr::null(),
            );
            if mapping.is_null() {
                return None;
            }
            let view = ffi::MapViewOfFile(mapping, ffi::FILE_MAP_READ, 0, 0, len);
            // A view mantem o mapeamento vivo; o handle nao e mais preciso.
            ffi::CloseHandle(mapping);
            if view.is_null() {
                return None;
            }
            Some(FileView { ptr: view as *const u8, len })
        }
    }

    #[inline]
    fn file_unmap(view: FileView) {
        unsafe {
            ffi::UnmapViewOfFile(view.ptr.cast());
        }
    }
}

impl FileSystem for Win32 {
    fn file_open(path: &str, access: FileAccess) -> Option<FileHandle> {
        let path = wide_path(path)?;
        let (desired, disposition) = match access {
            FileAccess::Read => (ffi::GENERIC_READ, ffi::OPEN_EXISTING),
            FileAccess::Write => (ffi::GENERIC_WRITE, ffi::CREATE_ALWAYS),
        };
        let handle = unsafe {
            ffi::CreateFileW(
                path.as_ptr(),
                desired,
                SHARE_ALL,
                ptr::null_mut(),
                disposition,
                ffi::FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == ffi::INVALID_HANDLE_VALUE {
            return None;
        }
        Some(FileHandle(handle))
    }

    fn file_write_at(file: &FileHandle, offset: u64, data: &[u8]) -> bool {
        let mut done = 0;
        while done < data.len() {
            let size = (data.len() - done).min(IO_CHUNK) as ffi::DWORD;
            let mut overlapped = at(offset + done as u64);
            let mut written: ffi::DWORD = 0;
            let ok = unsafe {
                ffi::WriteFile(file.0, data[done..].as_ptr().cast(), size, &mut written, &mut overlapped)
            };
            if ok == 0 || written == 0 {
                return false;
            }
            done += written as usize;
        }
        true
    }

    fn file_delete(path: &str) -> bool {
        let Some(path) = wide_path(path) else {
            return false;
        };
        unsafe { ffi::DeleteFileW(path.as_ptr()) != 0 }
    }

    fn file_rename(from: &str, to: &str) -> bool {
        let (Some(from), Some(to)) = (wide_path(from), wide_path(to)) else {
            return false;
        };
        unsafe { ffi::MoveFileExW(from.as_ptr(), to.as_ptr(), ffi::MOVEFILE_REPLACE_EXISTING) != 0 }
    }

    fn file_copy(from: &str, to: &str) -> bool {
        let (Some(from), Some(to)) = (wide_path(from), wide_path(to)) else {
            return false;
        };
        unsafe { ffi::CopyFileW(from.as_ptr(), to.as_ptr(), 0) != 0 }
    }

    /// `FILETIME` da ultima escrita: intervalos de 100 ns desde 1601.
    fn file_modified(path: &str) -> Option<u64> {
        let path = wide_path(path)?;
        let mut data: ffi::WIN32_FILE_ATTRIBUTE_DATA = unsafe { mem::zeroed() };
        let ok = unsafe {
            ffi::GetFileAttributesExW(path.as_ptr(), ffi::GetFileExInfoStandard, (&raw mut data).cast())
        };
        if ok == 0 {
            return None;
        }
        let time = data.ftLastWriteTime;
        Some((time.dwHighDateTime as u64) << 32 | time.dwLowDateTime as u64)
    }

    /// Se ja existe algo com esse nome, so vale como sucesso se for um
    /// diretorio.
    fn directory_create(path: &str) -> bool {
        let Some(path) = wide_path(path) else {
            return false;
        };
        unsafe { ffi::CreateDirectoryW(path.as_ptr(), ptr::null_mut()) != 0 || is_directory(&path) }
    }

    fn directory_delete(path: &str) -> bool {
        let Some(path) = wide_path(path) else {
            return false;
        };
        unsafe { ffi::RemoveDirectoryW(path.as_ptr()) != 0 }
    }

    fn directory_list(path: &str, mut visit: impl FnMut(&str, EntryKind)) -> bool {
        // `FindFirstFileW` recebe um padrao: `path/*`. Caminho vazio e o
        // diretorio atual.
        let path = if path.is_empty() { "." } else { path };
        let mut pattern = [0u16; PATH_UNITS + 2];
        let Some(mut len) = wide::encode(path, &mut pattern[..PATH_UNITS - 1]) else {
            return false;
        };
        if !path.ends_with('/') && !path.ends_with('\\') {
            pattern[len] = b'/' as u16;
            len += 1;
        }
        pattern[len] = b'*' as u16;

        let mut data: ffi::WIN32_FIND_DATAW = unsafe { mem::zeroed() };
        let find = unsafe { ffi::FindFirstFileW(pattern.as_ptr(), &mut data) };
        if find == ffi::INVALID_HANDLE_VALUE {
            return false;
        }

        // Nome de ate `MAX_PATH` unidades; cada uma vira no maximo 3 bytes.
        let mut name = [0u8; ffi::MAX_PATH * 3];
        loop {
            let units = until_nul(&data.cFileName);
            if !matches!(units, [DOT] | [DOT, DOT]) {
                if let Some(len) = wide::decode(units, &mut name) {
                    let kind = if data.dwFileAttributes & ffi::FILE_ATTRIBUTE_DIRECTORY != 0 {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    };
                    visit(unsafe { str::from_utf8_unchecked(&name[..len]) }, kind);
                }
            }
            if unsafe { ffi::FindNextFileW(find, &mut data) } == 0 {
                break;
            }
        }
        unsafe {
            ffi::FindClose(find);
        }
        true
    }

    fn path_executable(out: &mut [u8]) -> Option<&str> {
        let mut buf = [0u16; PATH_UNITS];
        let len = unsafe {
            ffi::GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), PATH_UNITS as ffi::DWORD)
        } as usize;
        // Truncado, devolve o tamanho do buffer inteiro.
        if len == 0 || len >= PATH_UNITS {
            return None;
        }
        path_out(&buf[..len], out)
    }

    /// `%APPDATA%` (Roaming), o equivalente ao `$XDG_DATA_HOME` do Linux.
    fn path_user_data(out: &mut [u8]) -> Option<&str> {
        unsafe {
            let mut path: ffi::PWSTR = ptr::null_mut();
            let result = ffi::SHGetKnownFolderPath(
                &ffi::FOLDERID_RoamingAppData,
                ffi::KF_FLAG_DEFAULT,
                ptr::null_mut(),
                &mut path,
            );
            let text = if result == ffi::S_OK && !path.is_null() {
                let mut len = 0;
                while *path.add(len) != 0 {
                    len += 1;
                }
                path_out(slice::from_raw_parts(path, len), out)
            } else {
                None
            };
            // Liberado mesmo quando falha; nulo e ignorado.
            ffi::CoTaskMemFree(path.cast());
            text
        }
    }
}
