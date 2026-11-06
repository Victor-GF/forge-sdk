#![allow(non_upper_case_globals)]

use super::shtypes::KNOWNFOLDERID;

/// `{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}`: `%APPDATA%`.
pub const FOLDERID_RoamingAppData: KNOWNFOLDERID = KNOWNFOLDERID {
    Data1: 0x3EB6_85DB,
    Data2: 0x65F9,
    Data3: 0x4CF6,
    Data4: [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
};
