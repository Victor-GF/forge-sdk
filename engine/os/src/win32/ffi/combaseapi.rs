use super::guiddef::{REFCLSID, REFIID};
use super::minwindef::{DWORD, LPVOID};
use super::unknwn::LPUNKNOWN;
use super::winnt::HRESULT;

/// `COINIT` (objbase.h): objetos COM usaveis de qualquer thread do processo.
pub const COINIT_MULTITHREADED: DWORD = 0x0;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn CoTaskMemFree(pv: LPVOID);
    pub fn CoInitializeEx(pvReserved: LPVOID, dwCoInit: DWORD) -> HRESULT;
    pub fn CoUninitialize();
    pub fn CoCreateInstance(
        rclsid: REFCLSID,
        pUnkOuter: LPUNKNOWN,
        dwClsContext: DWORD,
        riid: REFIID,
        ppv: *mut LPVOID,
    ) -> HRESULT;
}
