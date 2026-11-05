use super::minwindef::{BOOL, DWORD};
use super::winnt::LPCWSTR;

pub const INFINITE: DWORD = 0xFFFF_FFFF;

pub const STD_OUTPUT_HANDLE: DWORD = -11i32 as DWORD;

pub const MOVEFILE_REPLACE_EXISTING: DWORD = 0x0000_0001;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn MoveFileExW(lpExistingFileName: LPCWSTR, lpNewFileName: LPCWSTR, dwFlags: DWORD) -> BOOL;
    pub fn CopyFileW(lpExistingFileName: LPCWSTR, lpNewFileName: LPCWSTR, bFailIfExists: BOOL) -> BOOL;
}
