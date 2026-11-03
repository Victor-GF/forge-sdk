use super::{Win32, ffi};
use crate::memory::Memory;
use crate::thread::{Thread, ThreadEntry, ThreadHandle};
use core::ffi::c_void;
use core::{mem, ptr};

/// O que a thread nova precisa para chamar `entry`. `CreateThread` so passa
/// um ponteiro, entao os dois vao num bloco do heap que a propria thread
/// libera ao comecar.
struct ThreadStart {
    entry: ThreadEntry,
    param: *mut c_void,
}

unsafe extern "system" fn thread_trampoline(start: ffi::LPVOID) -> ffi::DWORD {
    unsafe {
        let ThreadStart { entry, param } = ptr::read(start as *const ThreadStart);
        Win32::heap_free(start.cast());
        entry(param)
    }
}

impl Thread for Win32 {
    fn thread_create(entry: ThreadEntry, param: *mut c_void) -> Option<ThreadHandle> {
        let start = Win32::heap_alloc(mem::size_of::<ThreadStart>()) as *mut ThreadStart;
        if start.is_null() {
            return None;
        }
        unsafe {
            start.write(ThreadStart { entry, param });
            let handle = ffi::CreateThread(ptr::null_mut(), 0, thread_trampoline, start.cast(), 0, ptr::null_mut());
            if handle.is_null() {
                Win32::heap_free(start.cast());
                return None;
            }
            Some(ThreadHandle(handle))
        }
    }

    fn thread_join(thread: ThreadHandle) -> u32 {
        unsafe {
            ffi::WaitForSingleObject(thread.0, ffi::INFINITE);
            let mut code: ffi::DWORD = 0;
            ffi::GetExitCodeThread(thread.0, &mut code);
            ffi::CloseHandle(thread.0);
            code
        }
    }

    #[inline]
    fn thread_detach(thread: ThreadHandle) {
        unsafe {
            ffi::CloseHandle(thread.0);
        }
    }

    #[inline]
    fn thread_current_id() -> u32 {
        unsafe { ffi::GetCurrentThreadId() }
    }

    #[inline]
    fn thread_sleep(ms: u32) {
        unsafe { ffi::Sleep(ms) }
    }

    #[inline]
    fn thread_yield() {
        unsafe {
            ffi::SwitchToThread();
        }
    }

    fn logical_processor_count() -> u32 {
        unsafe {
            let mut info: ffi::SYSTEM_INFO = mem::zeroed();
            ffi::GetSystemInfo(&mut info);
            info.dwNumberOfProcessors
        }
    }
}
