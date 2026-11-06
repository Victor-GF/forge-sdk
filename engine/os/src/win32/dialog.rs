use super::{Win32, ffi, wide};
use crate::dialog::Dialog;
use core::ptr;

/// Unidades UTF-16 do titulo e do texto, contando o nulo; mais que isso e
/// cortado.
const CAPTION_UNITS: usize = 256;
const TEXT_UNITS: usize = 2048;

impl Dialog for Win32 {
    fn message_box(caption: &str, text: &str) {
        let caption = wide::truncated::<CAPTION_UNITS>(caption);
        let text = wide::truncated::<TEXT_UNITS>(text);
        unsafe {
            ffi::MessageBoxW(
                ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                ffi::MB_OK | ffi::MB_ICONINFORMATION,
            );
        }
    }
}
