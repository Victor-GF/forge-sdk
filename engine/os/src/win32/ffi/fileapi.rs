use super::minwinbase::{GET_FILEEX_INFO_LEVELS, LPOVERLAPPED, LPSECURITY_ATTRIBUTES, LPWIN32_FIND_DATAW};
use super::minwindef::{BOOL, DWORD, FILETIME, LPCVOID, LPDWORD, LPVOID};
use super::winnt::{HANDLE, LPCWSTR, PLARGE_INTEGER};
use forge_core::layout;

pub const CREATE_ALWAYS: DWORD = 2;
pub const OPEN_EXISTING: DWORD = 3;

pub const INVALID_FILE_ATTRIBUTES: DWORD = 0xFFFF_FFFF;

#[repr(C)]
pub struct WIN32_FILE_ATTRIBUTE_DATA {
    pub dwFileAttributes: DWORD,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub nFileSizeHigh: DWORD,
    pub nFileSizeLow: DWORD,
}

layout!(WIN32_FILE_ATTRIBUTE_DATA, size = 36, align = 4);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn CreateFileW(
        lpFileName: LPCWSTR,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: LPSECURITY_ATTRIBUTES,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    pub fn ReadFile(
        hFile: HANDLE,
        lpBuffer: LPVOID,
        nNumberOfBytesToRead: DWORD,
        lpNumberOfBytesRead: LPDWORD,
        lpOverlapped: LPOVERLAPPED,
    ) -> BOOL;
    pub fn WriteFile(
        hFile: HANDLE,
        lpBuffer: LPCVOID,
        nNumberOfBytesToWrite: DWORD,
        lpNumberOfBytesWritten: LPDWORD,
        lpOverlapped: LPOVERLAPPED,
    ) -> BOOL;
    pub fn FlushFileBuffers(hFile: HANDLE) -> BOOL;
    pub fn GetFileSizeEx(hFile: HANDLE, lpFileSize: PLARGE_INTEGER) -> BOOL;
    pub fn GetFileAttributesW(lpFileName: LPCWSTR) -> DWORD;
    pub fn GetFileAttributesExW(
        lpFileName: LPCWSTR,
        fInfoLevelId: GET_FILEEX_INFO_LEVELS,
        lpFileInformation: LPVOID,
    ) -> BOOL;
    pub fn DeleteFileW(lpFileName: LPCWSTR) -> BOOL;
    pub fn CreateDirectoryW(lpPathName: LPCWSTR, lpSecurityAttributes: LPSECURITY_ATTRIBUTES) -> BOOL;
    pub fn RemoveDirectoryW(lpPathName: LPCWSTR) -> BOOL;
    pub fn FindFirstFileW(lpFileName: LPCWSTR, lpFindFileData: LPWIN32_FIND_DATAW) -> HANDLE;
    pub fn FindNextFileW(hFindFile: HANDLE, lpFindFileData: LPWIN32_FIND_DATAW) -> BOOL;
    pub fn FindClose(hFindFile: HANDLE) -> BOOL;
}
