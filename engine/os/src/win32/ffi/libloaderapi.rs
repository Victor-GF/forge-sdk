use super::minwindef::HMODULE;
use super::winnt::LPCSTR;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetModuleHandleA(lpModuleName: LPCSTR) -> HMODULE;
}
