use super::minwindef::BOOL;
use super::winnt::LPCWSTR;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn OutputDebugStringW(lpOutputString: LPCWSTR);
    pub fn IsDebuggerPresent() -> BOOL;
    pub fn DebugBreak();
}
