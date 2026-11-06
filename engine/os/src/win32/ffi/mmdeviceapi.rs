#![allow(non_upper_case_globals)]

use super::guiddef::{CLSID, IID, REFIID};
use super::minwindef::DWORD;
use super::unknwn::IUnknownVtbl;
use super::winnt::HRESULT;
use core::ffi::c_void;
use forge_core::layout;

/// Enum do SDK: direcao do dispositivo.
pub type EDataFlow = i32;
pub const eRender: EDataFlow = 0;

/// Enum do SDK: papel do dispositivo padrao.
pub type ERole = i32;
pub const eConsole: ERole = 0;

/// `{BCDE0395-E52F-467C-8E3D-C4579291692E}`
pub const CLSID_MMDeviceEnumerator: CLSID = CLSID {
    Data1: 0xBCDE_0395,
    Data2: 0xE52F,
    Data3: 0x467C,
    Data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
};

/// `{A95664D2-9614-4F35-A746-DE8DB63617E6}`
pub const IID_IMMDeviceEnumerator: IID = IID {
    Data1: 0xA956_64D2,
    Data2: 0x9614,
    Data3: 0x4F35,
    Data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
};

#[repr(C)]
pub struct IMMDeviceEnumerator {
    pub lpVtbl: *const IMMDeviceEnumeratorVtbl,
}

#[repr(C)]
pub struct IMMDeviceEnumeratorVtbl {
    pub base: IUnknownVtbl,
    pub EnumAudioEndpoints: usize,
    pub GetDefaultAudioEndpoint: unsafe extern "system" fn(
        This: *mut IMMDeviceEnumerator,
        dataFlow: EDataFlow,
        role: ERole,
        ppEndpoint: *mut *mut IMMDevice,
    ) -> HRESULT,
    pub GetDevice: usize,
    pub RegisterEndpointNotificationCallback: usize,
    pub UnregisterEndpointNotificationCallback: usize,
}

layout!(IMMDeviceEnumeratorVtbl, size = 64, align = 8);

#[repr(C)]
pub struct IMMDevice {
    pub lpVtbl: *const IMMDeviceVtbl,
}

#[repr(C)]
pub struct IMMDeviceVtbl {
    pub base: IUnknownVtbl,
    /// `pActivationParams` e um `PROPVARIANT*`; so passamos nulo.
    pub Activate: unsafe extern "system" fn(
        This: *mut IMMDevice,
        iid: REFIID,
        dwClsCtx: DWORD,
        pActivationParams: *mut c_void,
        ppInterface: *mut *mut c_void,
    ) -> HRESULT,
    pub OpenPropertyStore: usize,
    pub GetId: usize,
    pub GetState: usize,
}

layout!(IMMDeviceVtbl, size = 56, align = 8);
