use super::minwinbase::LPOVERLAPPED;
use super::minwindef::{BOOL, DWORD, LPCVOID, LPDWORD};
use super::winnt::HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn WriteFile(
        hFile: HANDLE,
        lpBuffer: LPCVOID,
        nNumberOfBytesToWrite: DWORD,
        lpNumberOfBytesWritten: LPDWORD,
        lpOverlapped: LPOVERLAPPED,
    ) -> BOOL;
}
