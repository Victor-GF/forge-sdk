use super::{Win32, ffi};
use crate::sync::{Mutex, SemaphoreHandle, Synchronization};
use core::sync::atomic::AtomicU32;
use core::{mem, ptr};

// `Mutex` guarda um `SRWLOCK` (um ponteiro, zero quando livre) no lugar do
// `usize`.
const _: () = assert!(
    mem::size_of::<Mutex>() == mem::size_of::<ffi::SRWLOCK>()
        && mem::align_of::<Mutex>() == mem::align_of::<ffi::SRWLOCK>()
);

#[inline]
fn srw(mutex: &Mutex) -> ffi::PSRWLOCK {
    mutex.0.get().cast()
}

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

    #[inline]
    fn mutex_lock(mutex: &Mutex) {
        unsafe { ffi::AcquireSRWLockExclusive(srw(mutex)) }
    }

    #[inline]
    fn mutex_try_lock(mutex: &Mutex) -> bool {
        unsafe { ffi::TryAcquireSRWLockExclusive(srw(mutex)) != 0 }
    }

    #[inline]
    unsafe fn mutex_unlock(mutex: &Mutex) {
        unsafe { ffi::ReleaseSRWLockExclusive(srw(mutex)) }
    }

    /// `WaitOnAddress` compara os 4 bytes de `address` com os de `expected`
    /// e so dorme se forem iguais. O `BOOL` de retorno so diz se houve
    /// timeout, e aqui nao ha.
    #[inline]
    fn address_wait(address: &AtomicU32, expected: u32) {
        unsafe {
            ffi::WaitOnAddress(
                address.as_ptr().cast(),
                (&raw const expected).cast(),
                mem::size_of::<u32>(),
                ffi::INFINITE,
            );
        }
    }

    #[inline]
    fn address_wake_one(address: &AtomicU32) {
        unsafe { ffi::WakeByAddressSingle(address.as_ptr().cast()) }
    }

    #[inline]
    fn address_wake_all(address: &AtomicU32) {
        unsafe { ffi::WakeByAddressAll(address.as_ptr().cast()) }
    }
}
