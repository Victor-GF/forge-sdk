use super::{Win32, ffi};
use crate::sync::{SemaphoreHandle, Synchronization};
use core::ptr;

impl Synchronization for Win32 {
    fn semaphore_create(initial: u32, max: u32) -> Option<SemaphoreHandle> {
        let handle = unsafe {
            ffi::CreateSemaphoreW(ptr::null_mut(), initial as ffi::LONG, max as ffi::LONG, ptr::null())
        };
        if handle.is_null() {
            return None;
        }
        Some(SemaphoreHandle(handle))
    }

    #[inline]
    fn semaphore_wait(semaphore: &SemaphoreHandle) {
        unsafe {
            ffi::WaitForSingleObject(semaphore.0, ffi::INFINITE);
        }
    }

    #[inline]
    fn semaphore_signal(semaphore: &SemaphoreHandle, count: u32) {
        unsafe {
            ffi::ReleaseSemaphore(semaphore.0, count as ffi::LONG, ptr::null_mut());
        }
    }

    #[inline]
    fn semaphore_destroy(semaphore: SemaphoreHandle) {
        unsafe {
            ffi::CloseHandle(semaphore.0);
        }
    }
}
