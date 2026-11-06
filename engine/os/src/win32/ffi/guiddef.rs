use forge_core::layout;

#[repr(C)]
pub struct GUID {
    pub Data1: u32,
    pub Data2: u16,
    pub Data3: u16,
    pub Data4: [u8; 8],
}

layout!(GUID, size = 16, align = 4);

pub type IID = GUID;
pub type CLSID = GUID;
pub type REFIID = *const IID;
pub type REFCLSID = *const CLSID;
pub type LPCGUID = *const GUID;
