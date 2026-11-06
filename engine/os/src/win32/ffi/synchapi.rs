use super::minwinbase::LPSECURITY_ATTRIBUTES;
use super::basetsd::SIZE_T;
use super::minwindef::{BOOL, BOOLEAN, DWORD, LPLONG, LPVOID};
use super::winnt::{HANDLE, LARGE_INTEGER, LONG, LPCWSTR, RTL_SRWLOCK};
use core::ffi::c_void;

/// Windows 10 1803 em diante; antes, `CreateWaitableTimerExW` falha com ela.
pub const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: DWORD = 0x0000_0002;

pub type SRWLOCK = RTL_SRWLOCK;
pub type PSRWLOCK = *mut SRWLOCK;

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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn AcquireSRWLockExclusive(SRWLock: PSRWLOCK);
    pub fn TryAcquireSRWLockExclusive(SRWLock: PSRWLOCK) -> BOOLEAN;
    pub fn ReleaseSRWLockExclusive(SRWLock: PSRWLOCK);
}

// Windows 8 em diante. O kernel32 nao exporta estas: elas vem do API set
// `api-ms-win-core-synch-l1-2-0`, que o Windows resolve para o KernelBase.
#[link(name = "api-ms-win-core-synch-l1-2-0", kind = "raw-dylib")]
unsafe extern "system" {
    /// `Address` e `volatile` no SDK.
    pub fn WaitOnAddress(Address: *const c_void, CompareAddress: *const c_void, AddressSize: SIZE_T, dwMilliseconds: DWORD) -> BOOL;
    pub fn WakeByAddressSingle(Address: *const c_void);
    pub fn WakeByAddressAll(Address: *const c_void);
}
