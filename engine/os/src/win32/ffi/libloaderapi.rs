use super::minwindef::{BOOL, DWORD, FARPROC, HMODULE};
use super::winnt::{LPCSTR, LPCWSTR, LPWSTR};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetModuleHandleA(lpModuleName: LPCSTR) -> HMODULE;
    pub fn GetModuleFileNameW(hModule: HMODULE, lpFilename: LPWSTR, nSize: DWORD) -> DWORD;
    pub fn LoadLibraryW(lpLibFileName: LPCWSTR) -> HMODULE;
    pub fn GetProcAddress(hModule: HMODULE, lpProcName: LPCSTR) -> FARPROC;
    pub fn FreeLibrary(hLibModule: HMODULE) -> BOOL;
}
