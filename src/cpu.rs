//! Hardware floor.
//!
//! On x86-64 this repository builds for **x86-64-v3**: AVX, AVX2, FMA, BMI1/2, F16C, LZCNT,
//! MOVBE (Intel Haswell 2013+, AMD Excavator/Zen 2015+). See `.cargo/config.toml`. A binary
//! built that way crashes with "illegal instruction" on older CPUs, so `check_floor` (called by
//! `JobSystem::new`) asks the CPU directly and exits with a readable message instead.
//!
//! Note: `is_x86_feature_detected!` can't be used for this — when a feature is enabled at compile
//! time the macro folds to `true` without asking the CPU.

/// Features this build assumes, or "" when no floor is compiled in.
pub fn required() -> &'static str {
    if cfg!(all(target_arch = "x86_64", target_feature = "avx2", target_feature = "fma")) {
        "x86-64-v3 (AVX2 + FMA + BMI2)"
    } else {
        ""
    }
}

#[cfg(target_arch = "x86_64")]
#[cfg_attr(not(target_feature = "avx2"), allow(dead_code))]
fn missing_features() -> Vec<&'static str> {
    use std::arch::x86_64::{__cpuid, __cpuid_count, _xgetbv};
    let (l1, l7) = (__cpuid(1), __cpuid_count(7, 0));
    let bit = |reg: u32, b: u32| reg >> b & 1 != 0;
    let osxsave = bit(l1.ecx, 27);
    // SAFETY: xgetbv only runs when OSXSAVE says the OS enabled it.
    let ymm_state = osxsave && unsafe { _xgetbv(0) } & 0b110 == 0b110;
    let mut missing = Vec::new();
    for (ok, name) in [
        (bit(l1.ecx, 28) && ymm_state, "AVX (with OS support)"),
        (bit(l1.ecx, 12), "FMA"),
        (bit(l7.ebx, 5), "AVX2"),
        (bit(l7.ebx, 3), "BMI1"),
        (bit(l7.ebx, 8), "BMI2"),
    ] {
        if !ok {
            missing.push(name);
        }
    }
    missing
}

/// Exits the process with an explanation if this CPU is below the compiled-in floor.
pub fn check_floor() {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        let missing = missing_features();
        if !missing.is_empty() {
            eprintln!(
                "forja: this build requires an {} CPU; missing: {}.\n\
                 Rebuild without `-C target-cpu=x86-64-v3` (see .cargo/config.toml) to run on this machine.",
                required(),
                missing.join(", ")
            );
            std::process::exit(1);
        }
    }
}

/// One cache of the CPU running this code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cache {
    pub level: u8,
    /// "data", "instruction" or "unified".
    pub kind: &'static str,
    pub bytes: usize,
    pub line: usize,
    /// How many hardware threads share one instance of this cache.
    pub shared_by: u32,
}

/// Caches as the CPU reports them (cpuid leaf 4 on Intel, 0x8000001D on AMD). Empty elsewhere.
pub fn caches() -> Vec<Cache> {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{__cpuid, __cpuid_count};
        let v = __cpuid(0);
        let amd = v.ebx == u32::from_le_bytes(*b"Auth");
        let leaf = if amd { 0x8000_001D } else { 4 };
        if amd && __cpuid(0x8000_0000).eax < leaf {
            return Vec::new();
        }
        let mut out = Vec::new();
        for sub in 0..16 {
            let r = __cpuid_count(leaf, sub);
            let kind = match r.eax & 0x1f {
                0 => break,
                1 => "data",
                2 => "instruction",
                _ => "unified",
            };
            let line = (r.ebx & 0xfff) as usize + 1;
            let partitions = ((r.ebx >> 12) & 0x3ff) as usize + 1;
            let ways = (r.ebx >> 22) as usize + 1;
            let sets = r.ecx as usize + 1;
            out.push(Cache {
                level: ((r.eax >> 5) & 0x7) as u8,
                kind,
                bytes: ways * partitions * line * sets,
                line,
                shared_by: ((r.eax >> 14) & 0xfff) + 1,
            });
        }
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    Vec::new()
}

/// CPU model name, e.g. "Intel(R) Core(TM) i7-7700 CPU @ 3.60GHz".
pub fn brand() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;
        if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
            let mut bytes = Vec::with_capacity(48);
            for leaf in 0x8000_0002..=0x8000_0004u32 {
                let r = __cpuid(leaf);
                for reg in [r.eax, r.ebx, r.ecx, r.edx] {
                    bytes.extend_from_slice(&reg.to_le_bytes());
                }
            }
            return String::from_utf8_lossy(&bytes).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        }
    }
    std::env::consts::ARCH.to_string()
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    #[test]
    fn reports_a_data_cache_hierarchy() {
        let caches = super::caches();
        let l1 = caches.iter().find(|c| c.level == 1 && c.kind == "data").expect("L1d");
        assert!(l1.bytes >= 16 << 10 && l1.line == 64, "{caches:?}");
        assert!(caches.iter().any(|c| c.level >= 2 && c.bytes > l1.bytes));
        assert!(!super::brand().is_empty());
    }

    #[test]
    fn this_machine_meets_the_floor() {
        if !super::required().is_empty() {
            assert!(super::missing_features().is_empty());
        }
    }
}
