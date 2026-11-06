#![allow(non_upper_case_globals)]

use super::basetsd::KAFFINITY;
use super::minwindef::{BYTE, DWORD, WORD};
use core::ffi::{c_char, c_void};
use forge_core::layout;

pub type LONG = i32;
pub type SHORT = i16;
pub type LONGLONG = i64;
pub type HRESULT = LONG;
pub type WCHAR = u16;
pub type HANDLE = *mut c_void;
pub type LPCSTR = *const c_char;
pub type LPCWSTR = *const WCHAR;
pub type LPWSTR = *mut WCHAR;
pub type PWSTR = *mut WCHAR;

layout!(HANDLE, size = 8, align = 8);

/// No SDK e uma union `QuadPart` / `{ LowPart, HighPart }`, resto da era de
/// 32 bits; aqui fica so `QuadPart`, com o mesmo layout.
#[repr(C)]
pub struct LARGE_INTEGER {
    pub QuadPart: LONGLONG,
}

layout!(LARGE_INTEGER, size = 8, align = 8);

pub type PLARGE_INTEGER = *mut LARGE_INTEGER;

pub const GENERIC_READ: DWORD = 0x8000_0000;
pub const GENERIC_WRITE: DWORD = 0x4000_0000;

pub const FILE_SHARE_READ: DWORD = 0x0000_0001;
pub const FILE_SHARE_WRITE: DWORD = 0x0000_0002;
pub const FILE_SHARE_DELETE: DWORD = 0x0000_0004;

pub const FILE_ATTRIBUTE_DIRECTORY: DWORD = 0x0000_0010;
pub const FILE_ATTRIBUTE_NORMAL: DWORD = 0x0000_0080;

pub const MEM_COMMIT: DWORD = 0x0000_1000;
pub const MEM_RESERVE: DWORD = 0x0000_2000;
pub const MEM_DECOMMIT: DWORD = 0x0000_4000;
pub const MEM_RELEASE: DWORD = 0x0000_8000;

pub const PAGE_NOACCESS: DWORD = 0x01;
pub const PAGE_READONLY: DWORD = 0x02;
pub const PAGE_READWRITE: DWORD = 0x04;

pub const HEAP_ZERO_MEMORY: DWORD = 0x0000_0008;

pub const TIMER_ALL_ACCESS: DWORD = 0x001F_0003;

// ---- Topologia do processador -------------------------------------------

/// Enum do SDK.
pub type LOGICAL_PROCESSOR_RELATIONSHIP = i32;
pub const RelationProcessorCore: LOGICAL_PROCESSOR_RELATIONSHIP = 0;
pub const RelationNumaNode: LOGICAL_PROCESSOR_RELATIONSHIP = 1;
pub const RelationCache: LOGICAL_PROCESSOR_RELATIONSHIP = 2;
pub const RelationProcessorPackage: LOGICAL_PROCESSOR_RELATIONSHIP = 3;
pub const RelationGroup: LOGICAL_PROCESSOR_RELATIONSHIP = 4;
pub const RelationAll: LOGICAL_PROCESSOR_RELATIONSHIP = 0xFFFF;

/// Enum do SDK.
pub type PROCESSOR_CACHE_TYPE = i32;
pub const CacheUnified: PROCESSOR_CACHE_TYPE = 0;
pub const CacheInstruction: PROCESSOR_CACHE_TYPE = 1;
pub const CacheData: PROCESSOR_CACHE_TYPE = 2;
pub const CacheTrace: PROCESSOR_CACHE_TYPE = 3;

/// `PROCESSOR_RELATIONSHIP::Flags`: o nucleo tem mais de uma thread logica.
pub const LTP_PC_SMT: BYTE = 0x1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GROUP_AFFINITY {
    pub Mask: KAFFINITY,
    pub Group: WORD,
    pub Reserved: [WORD; 3],
}

layout!(GROUP_AFFINITY, size = 16, align = 8);

/// `GroupMask` e um array de tamanho variavel (`GroupCount` itens); aqui so
/// o primeiro, que e o que um nucleo usa.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PROCESSOR_RELATIONSHIP {
    pub Flags: BYTE,
    pub EfficiencyClass: BYTE,
    pub Reserved: [BYTE; 20],
    pub GroupCount: WORD,
    pub GroupMask: [GROUP_AFFINITY; 1],
}

layout!(PROCESSOR_RELATIONSHIP, size = 40, align = 8);

/// No SDK, `GroupMask` esta numa union com o array `GroupMasks`; aqui fica so
/// o primeiro, com o mesmo layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CACHE_RELATIONSHIP {
    pub Level: BYTE,
    pub Associativity: BYTE,
    pub LineSize: WORD,
    pub CacheSize: DWORD,
    pub Type: PROCESSOR_CACHE_TYPE,
    pub Reserved: [BYTE; 18],
    pub GroupCount: WORD,
    pub GroupMask: GROUP_AFFINITY,
}

layout!(CACHE_RELATIONSHIP, size = 48, align = 8);

/// No SDK a union tambem tem `NumaNode` e `Group`, que nao usamos; sem eles,
/// o tamanho aqui e o do maior membro declarado. Cada entrada no buffer tem
/// o tamanho em `Size`, que e o que avanca para a proxima: nunca
/// `size_of` desta struct, nem ler ela inteira de uma vez.
#[repr(C)]
pub union SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX_0 {
    pub Processor: PROCESSOR_RELATIONSHIP,
    pub Cache: CACHE_RELATIONSHIP,
}

#[repr(C)]
pub struct SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX {
    pub Relationship: LOGICAL_PROCESSOR_RELATIONSHIP,
    pub Size: DWORD,
    pub u: SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX_0,
}

layout!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, size = 56, align = 8);

pub type PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX = *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;

// ---- SRW lock -----------------------------------------------------------

/// Um ponteiro de estado, zero quando livre (`RTL_SRWLOCK_INIT`).
#[repr(C)]
pub struct RTL_SRWLOCK {
    pub Ptr: *mut c_void,
}

layout!(RTL_SRWLOCK, size = 8, align = 8);
