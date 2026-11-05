use super::minwindef::DWORD;
use super::shtypes::REFKNOWNFOLDERID;
use super::winnt::{HANDLE, HRESULT, PWSTR};

/// `KNOWN_FOLDER_FLAG`: sem flags.
pub const KF_FLAG_DEFAULT: DWORD = 0;

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    /// `*ppszPath` precisa ser liberado com `CoTaskMemFree`, mesmo se falhar.
    pub fn SHGetKnownFolderPath(
        rfid: REFKNOWNFOLDERID,
        dwFlags: DWORD,
        hToken: HANDLE,
        ppszPath: *mut PWSTR,
    ) -> HRESULT;
}
