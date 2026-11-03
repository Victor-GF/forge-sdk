use super::minwindef::{BOOL, DWORD, LPVOID};
use forge_core::layout;

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: DWORD,
    pub lpSecurityDescriptor: LPVOID,
    pub bInheritHandle: BOOL,
}

layout!(SECURITY_ATTRIBUTES, size = 24, align = 8);

pub type LPSECURITY_ATTRIBUTES = *mut SECURITY_ATTRIBUTES;

pub type LPTHREAD_START_ROUTINE = unsafe extern "system" fn(lpThreadParameter: LPVOID) -> DWORD;
