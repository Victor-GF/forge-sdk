use super::{Win32, ffi, wide};
use crate::memory::Memory;
use crate::thread::{Thread, ThreadEntry, ThreadHandle, ThreadOptions, ThreadPriority};
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

/// Nome da thread no debugger, em UTF-16 com o nulo; mais que isso e cortado.
const NAME_UNITS: usize = 64;

/// Nome, prioridade e nucleo. So diagnostico e ajuste: se o SO recusar
/// algum, a thread segue sem ele.
unsafe fn apply_options(thread: ffi::HANDLE, options: &ThreadOptions) {
    unsafe {
        if !options.name.is_empty() {
            ffi::SetThreadDescription(thread, wide::truncated::<NAME_UNITS>(options.name).as_ptr());
        }
        let priority = match options.priority {
            ThreadPriority::Low => ffi::THREAD_PRIORITY_BELOW_NORMAL,
            ThreadPriority::Normal => ffi::THREAD_PRIORITY_NORMAL,
            ThreadPriority::High => ffi::THREAD_PRIORITY_ABOVE_NORMAL,
        };
        if priority != ffi::THREAD_PRIORITY_NORMAL {
            ffi::SetThreadPriority(thread, priority);
        }
        // A mascara de afinidade cobre os 64 processadores do grupo da
        // thread; acima disso o pedido e ignorado.
        if let Some(core) = options.core.filter(|&core| core < usize::BITS) {
            ffi::SetThreadAffinityMask(thread, 1usize << core);
        }
    }
}

impl Thread for Win32 {
    /// A thread nasce suspensa: nome, prioridade e nucleo valem desde a
    /// primeira instrucao, e so depois ela anda.
    fn thread_create(entry: ThreadEntry, param: *mut c_void, options: &ThreadOptions) -> Option<ThreadHandle> {
        let start = Win32::heap_alloc(mem::size_of::<ThreadStart>()) as *mut ThreadStart;
        if start.is_null() {
            return None;
        }
        unsafe {
            start.write(ThreadStart { entry, param });
            let mut flags = ffi::CREATE_SUSPENDED;
            if options.stack_size > 0 {
                flags |= ffi::STACK_SIZE_PARAM_IS_A_RESERVATION;
            }
            let handle = ffi::CreateThread(
                ptr::null_mut(),
                options.stack_size,
                thread_trampoline,
                start.cast(),
                flags,
                ptr::null_mut(),
            );
            if handle.is_null() {
                Win32::heap_free(start.cast());
                return None;
            }
            apply_options(handle, options);
            ffi::ResumeThread(handle);
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

    /// O nome e so diagnostico: se `SetThreadDescription` falhar, nada muda.
    fn thread_set_name(name: &str) {
        let name = wide::truncated::<NAME_UNITS>(name);
        unsafe {
            ffi::SetThreadDescription(ffi::GetCurrentThread(), name.as_ptr());
        }
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
