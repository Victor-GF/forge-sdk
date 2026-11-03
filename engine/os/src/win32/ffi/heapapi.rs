use super::basetsd::SIZE_T;
use super::minwindef::{BOOL, DWORD, LPVOID};
use super::winnt::HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetProcessHeap() -> HANDLE;
    pub fn HeapAlloc(hHeap: HANDLE, dwFlags: DWORD, dwBytes: SIZE_T) -> LPVOID;
    pub fn HeapReAlloc(hHeap: HANDLE, dwFlags: DWORD, lpMem: LPVOID, dwBytes: SIZE_T) -> LPVOID;
    pub fn HeapFree(hHeap: HANDLE, dwFlags: DWORD, lpMem: LPVOID) -> BOOL;
}
