use super::minwinbase::LPSECURITY_ATTRIBUTES;
use super::minwindef::{BOOL, DWORD, LPLONG};
use super::winnt::{HANDLE, LONG, LPCWSTR};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: DWORD) -> DWORD;
    pub fn Sleep(dwMilliseconds: DWORD);

    pub fn CreateSemaphoreW(
        lpSemaphoreAttributes: LPSECURITY_ATTRIBUTES,
        lInitialCount: LONG,
        lMaximumCount: LONG,
        lpName: LPCWSTR,
    ) -> HANDLE;
    pub fn ReleaseSemaphore(hSemaphore: HANDLE, lReleaseCount: LONG, lpPreviousCount: LPLONG) -> BOOL;
}
