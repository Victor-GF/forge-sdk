//! Captura de pilha e formatação `módulo+deslocamento`, sem alocar.
//!
//! O deslocamento é relativo ao início do módulo, então vale entre execuções (ASLR) e pode ser
//! simbolizado depois com `tools/symbolize.sh` no mesmo build.

use core::ffi::c_void;
use core::fmt::Write;
use core::ptr::null_mut;

use forja_core::text::TextBuf;
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace;
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};

pub const FRAMES_MAX: usize = 62;

/// Abaixo disso o Windows nunca mapeia código: endereços assim no fim da pilha são lixo do fim
/// da cadeia de quadros.
const LOWEST_CODE_ADDRESS: usize = 0x1_0000;

/// Preenche `frames` com os endereços de retorno da pilha atual, pulando `skip` quadros além
/// desta função. Devolve quantos foram capturados.
pub fn capture(skip: u32, frames: &mut [usize]) -> usize {
    let mut raw = [null_mut::<c_void>(); FRAMES_MAX];
    let wanted = u32::try_from(frames.len().min(FRAMES_MAX)).unwrap_or(0);
    // SAFETY: `raw` tem espaço para `FRAMES_MAX` ponteiros e `wanted <= FRAMES_MAX`.
    let n = unsafe { RtlCaptureStackBackTrace(skip.saturating_add(1), wanted, raw.as_mut_ptr(), null_mut()) };
    let mut count: usize = 0;
    for (dst, src) in frames.iter_mut().zip(raw.iter()).take(usize::from(n)) {
        let address = *src as usize;
        if address < LOWEST_CODE_ADDRESS {
            break;
        }
        *dst = address;
        count = count.saturating_add(1);
    }
    count
}

/// Escreve `racing.exe+0x1a2b`, ou o endereço cru se ele não pertencer a nenhum módulo.
pub fn write_module_offset(t: &mut TextBuf, address: usize) {
    let mut module: HMODULE = null_mut();
    // SAFETY: com FROM_ADDRESS o "nome" é um endereço qualquer; `module` é local.
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    if ok == 0 || module.is_null() {
        let _ = write!(t, "0x{address:x}");
        return;
    }
    let mut name = [0u16; 260];
    // SAFETY: `name` tem 260 posições.
    let n = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), 260) };
    let path = name.get(..n as usize).unwrap_or(&[]);
    let start = path.iter().rposition(|&c| c == u16::from(b'\\')).map_or(0, |i| i.saturating_add(1));
    let file = path.get(start..).unwrap_or(&[]);
    for c in char::decode_utf16(file.iter().copied()) {
        let _ = t.write_char(c.unwrap_or('?'));
    }
    let _ = write!(t, "+0x{:x}", address.wrapping_sub(module as usize));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_frames_inside_the_test_binary() {
        let mut frames = [0usize; 8];
        let n = capture(0, &mut frames);
        assert!(n > 0);
        let mut buf = [0u8; 128];
        let mut t = TextBuf::new(&mut buf);
        write_module_offset(&mut t, frames[0]);
        assert!(t.as_str().contains(".exe+0x"), "{}", t.as_str());
    }
}
