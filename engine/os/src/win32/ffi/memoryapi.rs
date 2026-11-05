use super::basetsd::SIZE_T;
use super::minwinbase::LPSECURITY_ATTRIBUTES;
use super::minwindef::{BOOL, DWORD, LPCVOID, LPVOID};
use super::winnt::{HANDLE, LPCWSTR};

pub const FILE_MAP_READ: DWORD = 0x0004;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn VirtualAlloc(
        lpAddress: LPVOID,
        dwSize: SIZE_T,
        flAllocationType: DWORD,
        flProtect: DWORD,
    ) -> LPVOID;
    pub fn VirtualFree(lpAddress: LPVOID, dwSize: SIZE_T, dwFreeType: DWORD) -> BOOL;
    pub fn CreateFileMappingW(
        hFile: HANDLE,
        lpFileMappingAttributes: LPSECURITY_ATTRIBUTES,
        flProtect: DWORD,
        dwMaximumSizeHigh: DWORD,
        dwMaximumSizeLow: DWORD,
        lpName: LPCWSTR,
    ) -> HANDLE;
    pub fn MapViewOfFile(
        hFileMappingObject: HANDLE,
        dwDesiredAccess: DWORD,
        dwFileOffsetHigh: DWORD,
        dwFileOffsetLow: DWORD,
        dwNumberOfBytesToMap: SIZE_T,
    ) -> LPVOID;
    pub fn UnmapViewOfFile(lpBaseAddress: LPCVOID) -> BOOL;
}
