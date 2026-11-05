use super::minwindef::DWORD;
use super::winnt::HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetStdHandle(nStdHandle: DWORD) -> HANDLE;
}
