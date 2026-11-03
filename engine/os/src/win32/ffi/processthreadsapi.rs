use super::basetsd::SIZE_T;
use super::minwinbase::{LPSECURITY_ATTRIBUTES, LPTHREAD_START_ROUTINE};
use super::minwindef::{BOOL, DWORD, LPDWORD, LPVOID, UINT};
use super::winnt::HANDLE;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn ExitProcess(uExitCode: UINT) -> !;

    pub fn CreateThread(
        lpThreadAttributes: LPSECURITY_ATTRIBUTES,
        dwStackSize: SIZE_T,
        lpStartAddress: LPTHREAD_START_ROUTINE,
        lpParameter: LPVOID,
        dwCreationFlags: DWORD,
        lpThreadId: LPDWORD,
    ) -> HANDLE;
    pub fn GetExitCodeThread(hThread: HANDLE, lpExitCode: LPDWORD) -> BOOL;
    pub fn GetCurrentThreadId() -> DWORD;
    pub fn SwitchToThread() -> BOOL;
}
