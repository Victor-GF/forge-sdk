use super::minwindef::BOOL;
use super::winnt::HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn CloseHandle(hObject: HANDLE) -> BOOL;
}
