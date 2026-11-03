use super::basetsd::DWORD_PTR;
use super::minwindef::{DWORD, LPVOID, WORD};
use forge_core::layout;

/// O primeiro campo no SDK e uma union `dwOemId` / `{ wProcessorArchitecture,
/// wReserved }`; aqui fica so a forma de struct, com o mesmo layout.
#[repr(C)]
pub struct SYSTEM_INFO {
    pub wProcessorArchitecture: WORD,
    pub wReserved: WORD,
    pub dwPageSize: DWORD,
    pub lpMinimumApplicationAddress: LPVOID,
    pub lpMaximumApplicationAddress: LPVOID,
    pub dwActiveProcessorMask: DWORD_PTR,
    pub dwNumberOfProcessors: DWORD,
    pub dwProcessorType: DWORD,
    pub dwAllocationGranularity: DWORD,
    pub wProcessorLevel: WORD,
    pub wProcessorRevision: WORD,
}

layout!(SYSTEM_INFO, size = 48, align = 8);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetSystemInfo(lpSystemInfo: *mut SYSTEM_INFO);
}
