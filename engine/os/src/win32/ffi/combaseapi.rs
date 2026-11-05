use super::minwindef::LPVOID;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn CoTaskMemFree(pv: LPVOID);
}
