use super::minwindef::BOOL;
use super::winnt::HANDLE;

pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn CloseHandle(hObject: HANDLE) -> BOOL;
}
