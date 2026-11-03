use super::basetsd::SIZE_T;
use super::minwindef::{BOOL, DWORD, LPVOID};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn VirtualAlloc(
        lpAddress: LPVOID,
        dwSize: SIZE_T,
        flAllocationType: DWORD,
        flProtect: DWORD,
    ) -> LPVOID;
    pub fn VirtualFree(lpAddress: LPVOID, dwSize: SIZE_T, dwFreeType: DWORD) -> BOOL;
}
