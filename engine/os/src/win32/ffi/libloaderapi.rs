use super::minwindef::{DWORD, HMODULE};
use super::winnt::{LPCSTR, LPWSTR};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetModuleHandleA(lpModuleName: LPCSTR) -> HMODULE;
    pub fn GetModuleFileNameW(hModule: HMODULE, lpFilename: LPWSTR, nSize: DWORD) -> DWORD;
}
