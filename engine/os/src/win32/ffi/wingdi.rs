use super::minwindef::{BOOL, BYTE, DWORD, INT, UINT, WORD};
use super::windef::HDC;
use super::winnt::LONG;
use core::ffi::c_void;
use forge_core::layout;

/// Sem compressao: pixels crus.
pub const BI_RGB: DWORD = 0;
/// `bmiColors` seria uma paleta de cores RGB; com 32 bits nao ha paleta.
pub const DIB_RGB_COLORS: UINT = 0;

/// Operacoes de raster: copiar a origem; pintar de preto.
pub const SRCCOPY: DWORD = 0x00CC_0020;
pub const BLACKNESS: DWORD = 0x0000_0042;

/// Modo de esticar: descarta linhas e colunas em vez de misturar. Para
/// aumentar, e vizinho mais proximo.
pub const COLORONCOLOR: INT = 3;

#[repr(C)]
pub struct BITMAPINFOHEADER {
    pub biSize: DWORD,
    pub biWidth: LONG,
    /// Negativo: linhas de cima para baixo.
    pub biHeight: LONG,
    pub biPlanes: WORD,
    pub biBitCount: WORD,
    pub biCompression: DWORD,
    pub biSizeImage: DWORD,
    pub biXPelsPerMeter: LONG,
    pub biYPelsPerMeter: LONG,
    pub biClrUsed: DWORD,
    pub biClrImportant: DWORD,
}

layout!(BITMAPINFOHEADER, size = 40, align = 4);

#[repr(C)]
pub struct RGBQUAD {
    pub rgbBlue: BYTE,
    pub rgbGreen: BYTE,
    pub rgbRed: BYTE,
    pub rgbReserved: BYTE,
}

layout!(RGBQUAD, size = 4, align = 1);

/// `bmiColors` e um array de tamanho variavel (a paleta); sem paleta, o
/// item do SDK fica sem uso.
#[repr(C)]
pub struct BITMAPINFO {
    pub bmiHeader: BITMAPINFOHEADER,
    pub bmiColors: [RGBQUAD; 1],
}

layout!(BITMAPINFO, size = 44, align = 4);

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn StretchDIBits(
        hdc: HDC,
        xDest: INT,
        yDest: INT,
        DestWidth: INT,
        DestHeight: INT,
        xSrc: INT,
        ySrc: INT,
        SrcWidth: INT,
        SrcHeight: INT,
        lpBits: *const c_void,
        lpbmi: *const BITMAPINFO,
        iUsage: UINT,
        rop: DWORD,
    ) -> INT;
    pub fn SetStretchBltMode(hdc: HDC, mode: INT) -> INT;
    pub fn PatBlt(hdc: HDC, x: INT, y: INT, w: INT, h: INT, rop: DWORD) -> BOOL;
}
