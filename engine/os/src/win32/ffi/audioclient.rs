#![allow(non_upper_case_globals)]

use super::audiosessiontypes::AUDCLNT_SHAREMODE;
use super::guiddef::{IID, LPCGUID, REFIID};
use super::minwindef::{BYTE, DWORD, UINT32};
use super::mmeapi::WAVEFORMATEX;
use super::unknwn::IUnknownVtbl;
use super::winnt::HRESULT;
use core::ffi::c_void;
use forge_core::layout;

/// Intervalos de 100 ns.
pub type REFERENCE_TIME = i64;

/// O dispositivo sumiu (desconectado, desativado, trocado).
pub const AUDCLNT_E_DEVICE_INVALIDATED: HRESULT = 0x8889_0004_u32 as HRESULT;

/// `{1CB9AD4C-DBFA-4C32-B178-C2F568A703B2}`
pub const IID_IAudioClient: IID = IID {
    Data1: 0x1CB9_AD4C,
    Data2: 0xDBFA,
    Data3: 0x4C32,
    Data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
};

/// `{F294ACFC-3146-4483-A7BF-ADDCA7C260E2}`
pub const IID_IAudioRenderClient: IID = IID {
    Data1: 0xF294_ACFC,
    Data2: 0x3146,
    Data3: 0x4483,
    Data4: [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2],
};

#[repr(C)]
pub struct IAudioClient {
    pub lpVtbl: *const IAudioClientVtbl,
}

#[repr(C)]
pub struct IAudioClientVtbl {
    pub base: IUnknownVtbl,
    pub Initialize: unsafe extern "system" fn(
        This: *mut IAudioClient,
        ShareMode: AUDCLNT_SHAREMODE,
        StreamFlags: DWORD,
        hnsBufferDuration: REFERENCE_TIME,
        hnsPeriodicity: REFERENCE_TIME,
        pFormat: *const WAVEFORMATEX,
        AudioSessionGuid: LPCGUID,
    ) -> HRESULT,
    pub GetBufferSize: unsafe extern "system" fn(This: *mut IAudioClient, pNumBufferFrames: *mut UINT32) -> HRESULT,
    pub GetStreamLatency: usize,
    pub GetCurrentPadding:
        unsafe extern "system" fn(This: *mut IAudioClient, pNumPaddingFrames: *mut UINT32) -> HRESULT,
    pub IsFormatSupported: usize,
    pub GetMixFormat: usize,
    pub GetDevicePeriod: usize,
    pub Start: unsafe extern "system" fn(This: *mut IAudioClient) -> HRESULT,
    pub Stop: unsafe extern "system" fn(This: *mut IAudioClient) -> HRESULT,
    pub Reset: usize,
    pub SetEventHandle: usize,
    pub GetService:
        unsafe extern "system" fn(This: *mut IAudioClient, riid: REFIID, ppv: *mut *mut c_void) -> HRESULT,
}

layout!(IAudioClientVtbl, size = 120, align = 8);

#[repr(C)]
pub struct IAudioRenderClient {
    pub lpVtbl: *const IAudioRenderClientVtbl,
}

#[repr(C)]
pub struct IAudioRenderClientVtbl {
    pub base: IUnknownVtbl,
    pub GetBuffer: unsafe extern "system" fn(
        This: *mut IAudioRenderClient,
        NumFramesRequested: UINT32,
        ppData: *mut *mut BYTE,
    ) -> HRESULT,
    pub ReleaseBuffer:
        unsafe extern "system" fn(This: *mut IAudioRenderClient, NumFramesWritten: UINT32, dwFlags: DWORD) -> HRESULT,
}

layout!(IAudioRenderClientVtbl, size = 40, align = 8);
