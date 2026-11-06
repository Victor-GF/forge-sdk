//! COM a mao: uma interface e uma struct cujo primeiro campo aponta para a
//! vtable, um array de ponteiros de funcao na ordem do header. Cada vtable
//! comeca com os tres metodos de `IUnknown`.
//!
//! Metodos que nao usamos ficam como `usize`: so ocupam a posicao, para os
//! seguintes cairem no lugar certo. A ordem e o tamanho de cada vtable sao
//! travados com `layout!`.

use super::guiddef::REFIID;
use super::minwindef::ULONG;
use super::winnt::HRESULT;
use core::ffi::c_void;
use forge_core::layout;

#[repr(C)]
pub struct IUnknown {
    pub lpVtbl: *const IUnknownVtbl,
}

pub type LPUNKNOWN = *mut IUnknown;

#[repr(C)]
pub struct IUnknownVtbl {
    pub QueryInterface:
        unsafe extern "system" fn(This: *mut IUnknown, riid: REFIID, ppvObject: *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(This: *mut IUnknown) -> ULONG,
    pub Release: unsafe extern "system" fn(This: *mut IUnknown) -> ULONG,
}

layout!(IUnknownVtbl, size = 24, align = 8);
