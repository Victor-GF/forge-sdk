use super::{Win32, ffi};
use crate::cpu::{Cpu, CpuTopology};
use crate::memory::Memory;
use core::{mem, ptr};

/// `Relationship` + `Size`: o que vem antes da union em cada entrada.
const HEADER: usize = mem::size_of::<ffi::LOGICAL_PROCESSOR_RELATIONSHIP>() + mem::size_of::<ffi::DWORD>();

impl Cpu for Win32 {
    /// Duas chamadas: a primeira so diz o tamanho do buffer.
    fn cpu_topology() -> Option<CpuTopology> {
        unsafe {
            let mut length: ffi::DWORD = 0;
            let measured = ffi::GetLogicalProcessorInformationEx(ffi::RelationAll, ptr::null_mut(), &mut length);
            if measured != 0 || ffi::GetLastError() != ffi::ERROR_INSUFFICIENT_BUFFER {
                return None;
            }
            let buffer = Win32::heap_alloc(length as usize);
            if buffer.is_null() {
                return None;
            }
            let ok = ffi::GetLogicalProcessorInformationEx(ffi::RelationAll, buffer.cast(), &mut length) != 0;
            let topology = if ok { Some(parse(buffer, length as usize)) } else { None };
            Win32::heap_free(buffer);
            topology
        }
    }
}

/// Percorre as entradas, de tamanho variavel (`Size`). Le campo a campo,
/// sem assumir alinhamento e sem ler alem da entrada.
unsafe fn parse(buffer: *const u8, length: usize) -> CpuTopology {
    let mut topology = CpuTopology::default();
    // Maior `EfficiencyClass` vista: maior e mais rapido.
    let mut fastest: Option<u8> = None;

    let mut offset = 0;
    while offset + HEADER <= length {
        unsafe {
            let entry = buffer.add(offset) as *const ffi::SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
            let relationship = ptr::read_unaligned(&raw const (*entry).Relationship);
            let size = ptr::read_unaligned(&raw const (*entry).Size) as usize;
            if size < HEADER || offset + size > length {
                break;
            }

            match relationship {
                ffi::RelationProcessorCore if size >= HEADER + mem::size_of::<ffi::PROCESSOR_RELATIONSHIP>() => {
                    let core = ptr::read_unaligned(&raw const (*entry).u.Processor);
                    topology.physical_cores += 1;
                    if core.GroupCount > 0 {
                        topology.logical_processors += core.GroupMask[0].Mask.count_ones();
                    }
                    match fastest {
                        Some(class) if core.EfficiencyClass < class => {}
                        Some(class) if core.EfficiencyClass == class => topology.performance_cores += 1,
                        _ => {
                            fastest = Some(core.EfficiencyClass);
                            topology.performance_cores = 1;
                        }
                    }
                }
                ffi::RelationCache if size >= HEADER + mem::size_of::<ffi::CACHE_RELATIONSHIP>() => {
                    let cache = ptr::read_unaligned(&raw const (*entry).u.Cache);
                    // Em CPU hibrida os tamanhos variam por tipo de nucleo:
                    // fica o maior.
                    match (cache.Level, cache.Type) {
                        (1, ffi::CacheData | ffi::CacheUnified) => {
                            topology.l1_data = topology.l1_data.max(cache.CacheSize);
                            topology.cache_line = topology.cache_line.max(cache.LineSize as u32);
                        }
                        (2, _) => topology.l2 = topology.l2.max(cache.CacheSize),
                        (3, _) => {
                            topology.l3 = topology.l3.max(cache.CacheSize);
                            topology.l3_count += 1;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            offset += size;
        }
    }
    topology
}
