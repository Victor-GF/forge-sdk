use super::{Win32, ffi};
use crate::time::Time;

// Desde o Windows XP, `QueryPerformanceCounter`/`QueryPerformanceFrequency`
// nunca falham: o `BOOL` de retorno e ignorado.
//
// Em hardware atual as duas rodam em user mode, sem syscall: o contador vem
// do TSC invariante da CPU e a frequencia de uma pagina que o kernel mapeia
// em todo processo. So em maquinas sem TSC confiavel o contador cai para um
// timer de hardware via kernel. Por isso a frequencia nao fica em cache em
// estado global.

impl Time for Win32 {
    #[inline]
    fn time_ticks() -> u64 {
        let mut count = ffi::LARGE_INTEGER { QuadPart: 0 };
        unsafe {
            ffi::QueryPerformanceCounter(&mut count);
        }
        count.QuadPart as u64
    }

    #[inline]
    fn time_frequency() -> u64 {
        let mut frequency = ffi::LARGE_INTEGER { QuadPart: 0 };
        unsafe {
            ffi::QueryPerformanceFrequency(&mut frequency);
        }
        frequency.QuadPart as u64
    }
}
