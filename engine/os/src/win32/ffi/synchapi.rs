use super::minwinbase::LPSECURITY_ATTRIBUTES;
use super::minwindef::{BOOL, DWORD, LPLONG, LPVOID};
use super::winnt::{HANDLE, LARGE_INTEGER, LONG, LPCWSTR};
use core::ffi::c_void;

/// Windows 10 1803 em diante; antes, `CreateWaitableTimerExW` falha com ela.
pub const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: DWORD = 0x0000_0002;

/// No SDK e um ponteiro de funcao que pode ser nulo; so passamos nulo.
pub type PTIMERAPCROUTINE = *mut c_void;

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

    pub fn CreateWaitableTimerExW(
        lpTimerAttributes: LPSECURITY_ATTRIBUTES,
        lpTimerName: LPCWSTR,
        dwFlags: DWORD,
        dwDesiredAccess: DWORD,
    ) -> HANDLE;
    pub fn SetWaitableTimer(
        hTimer: HANDLE,
        lpDueTime: *const LARGE_INTEGER,
        lPeriod: LONG,
        pfnCompletionRoutine: PTIMERAPCROUTINE,
        lpArgToCompletionRoutine: LPVOID,
        fResume: BOOL,
    ) -> BOOL;
}
