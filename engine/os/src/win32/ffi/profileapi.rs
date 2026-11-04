use super::minwindef::BOOL;
use super::winnt::LARGE_INTEGER;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn QueryPerformanceCounter(lpPerformanceCount: *mut LARGE_INTEGER) -> BOOL;
    pub fn QueryPerformanceFrequency(lpFrequency: *mut LARGE_INTEGER) -> BOOL;
}
