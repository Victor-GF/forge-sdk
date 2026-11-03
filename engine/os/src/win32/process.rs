use super::{Win32, ffi};
use crate::process::Process;

impl Process for Win32 {
    #[inline]
    fn exit_process(code: u32) -> ! {
        unsafe { ffi::ExitProcess(code) }
    }
}
