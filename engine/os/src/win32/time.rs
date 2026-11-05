use super::{Win32, ffi};
use crate::time::{Time, TimerHandle};
use core::ptr;

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

    /// Timer de alta resolucao do Windows 10 1803; antes dele, a criacao
    /// com a flag falha e o timer comum fica no lugar.
    fn timer_create() -> Option<TimerHandle> {
        unsafe {
            let mut timer = ffi::CreateWaitableTimerExW(
                ptr::null_mut(),
                ptr::null(),
                ffi::CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                ffi::TIMER_ALL_ACCESS,
            );
            if timer.is_null() {
                timer = ffi::CreateWaitableTimerExW(ptr::null_mut(), ptr::null(), 0, ffi::TIMER_ALL_ACCESS);
            }
            if timer.is_null() {
                return None;
            }
            Some(TimerHandle(timer))
        }
    }

    /// O timer recebe o tempo relativo em unidades de 100 ns, negativo
    /// (positivo seria data absoluta do relogio de parede, que pode mudar).
    /// Arredondado para cima: nao acorda antes do prazo por arredondamento.
    fn timer_wait_until(timer: &TimerHandle, deadline: u64) {
        let now = Win32::time_ticks();
        if now >= deadline {
            return;
        }
        let frequency = Win32::time_frequency() as u128;
        let units = ((deadline - now) as u128 * 10_000_000).div_ceil(frequency);
        let due = ffi::LARGE_INTEGER { QuadPart: -(units.min(i64::MAX as u128) as i64) };
        unsafe {
            if ffi::SetWaitableTimer(timer.0, &due, 0, ptr::null_mut(), ptr::null_mut(), 0) != 0 {
                ffi::WaitForSingleObject(timer.0, ffi::INFINITE);
            }
        }
    }

    #[inline]
    fn timer_destroy(timer: TimerHandle) {
        unsafe {
            ffi::CloseHandle(timer.0);
        }
    }
}
