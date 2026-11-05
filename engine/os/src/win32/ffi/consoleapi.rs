use super::minwindef::{BOOL, DWORD, LPDWORD, LPVOID};
use super::winnt::HANDLE;
use core::ffi::c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetConsoleMode(hConsoleHandle: HANDLE, lpMode: LPDWORD) -> BOOL;
    pub fn WriteConsoleW(
        hConsoleOutput: HANDLE,
        lpBuffer: *const c_void,
        nNumberOfCharsToWrite: DWORD,
        lpNumberOfCharsWritten: LPDWORD,
        lpReserved: LPVOID,
    ) -> BOOL;
}
