#![allow(non_upper_case_globals)]

use super::basetsd::ULONG_PTR;
use super::minwindef::{BOOL, DWORD, FILETIME, LPVOID, MAX_PATH};
use super::winnt::{HANDLE, WCHAR};
use forge_core::layout;

/// No SDK, `Offset`/`OffsetHigh` estao numa union com `Pointer: PVOID`; aqui
/// fica so a forma de struct, com o mesmo layout.
#[repr(C)]
pub struct OVERLAPPED {
    pub Internal: ULONG_PTR,
    pub InternalHigh: ULONG_PTR,
    pub Offset: DWORD,
    pub OffsetHigh: DWORD,
    pub hEvent: HANDLE,
}

layout!(OVERLAPPED, size = 32, align = 8);

pub type LPOVERLAPPED = *mut OVERLAPPED;

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: DWORD,
    pub lpSecurityDescriptor: LPVOID,
    pub bInheritHandle: BOOL,
}

layout!(SECURITY_ATTRIBUTES, size = 24, align = 8);

pub type LPSECURITY_ATTRIBUTES = *mut SECURITY_ATTRIBUTES;

#[repr(C)]
pub struct WIN32_FIND_DATAW {
    pub dwFileAttributes: DWORD,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub nFileSizeHigh: DWORD,
    pub nFileSizeLow: DWORD,
    pub dwReserved0: DWORD,
    pub dwReserved1: DWORD,
    pub cFileName: [WCHAR; MAX_PATH],
    pub cAlternateFileName: [WCHAR; 14],
}

layout!(WIN32_FIND_DATAW, size = 592, align = 4);

pub type LPWIN32_FIND_DATAW = *mut WIN32_FIND_DATAW;

/// Enum do SDK; so o valor usado.
pub type GET_FILEEX_INFO_LEVELS = i32;
pub const GetFileExInfoStandard: GET_FILEEX_INFO_LEVELS = 0;

pub type LPTHREAD_START_ROUTINE = unsafe extern "system" fn(lpThreadParameter: LPVOID) -> DWORD;
