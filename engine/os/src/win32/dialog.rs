use super::{Win32, ffi};
use crate::dialog::Dialog;
use core::ffi::CStr;
use core::ptr;

impl Dialog for Win32 {
    #[inline]
    fn message_box(caption: &CStr, text: &CStr) {
        unsafe {
            ffi::MessageBoxA(
                ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                ffi::MB_OK | ffi::MB_ICONINFORMATION,
            );
        }
    }
}
