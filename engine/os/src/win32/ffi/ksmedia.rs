use super::guiddef::GUID;

/// `{00000003-0000-0010-8000-00AA00389B71}`: amostras `f32`.
pub const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID {
    Data1: 0x0000_0003,
    Data2: 0x0000,
    Data3: 0x0010,
    Data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
};
