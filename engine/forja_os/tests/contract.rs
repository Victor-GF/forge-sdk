//! Testes de contrato do `forja_os`: as mesmas expectativas em todos os sistemas.
//! Só a API pública é usada. O handler de erro fatal é testado em `tests/crash` (processos
//! separados), porque derruba o processo.

// Arquivo só de teste: as mesmas exceções do clippy.toml valem para as funções auxiliares.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use forja_core::text::TextBuf;
use forja_os::file::{self, FileWriter, PATH_MAX};
use forja_os::{memory, process, stack, time};

#[test]
fn page_info_is_consistent() {
    let p = memory::page_info();
    assert!(p.page_size.is_power_of_two() && p.page_size >= 4096, "{p:?}");
    assert!(p.allocation_granularity.is_multiple_of(p.page_size), "{p:?}");
}

#[test]
fn reserve_gives_aligned_zeroed_writable_memory() {
    let p = memory::page_info();
    let size = 3 * p.page_size + 1;
    let base = memory::reserve(size).unwrap();
    assert!((base as usize).is_multiple_of(p.allocation_granularity));
    let bytes = unsafe { core::slice::from_raw_parts_mut(base, size) };
    assert!(bytes.iter().all(|&b| b == 0));
    bytes[0] = 1;
    bytes[size - 1] = 2;
    assert_eq!((bytes[0], bytes[size - 1]), (1, 2));
    assert!(memory::reserve(0).is_none());
}

#[test]
fn clock_is_monotonic_and_sleep_waits() {
    let tps = time::ticks_per_second();
    assert!(tps >= 1_000_000, "resolução baixa demais: {tps}");
    time::begin_precise_sleep();
    let a = time::ticks();
    time::sleep_ms(20);
    let b = time::ticks();
    time::end_precise_sleep();
    assert!(b >= a);
    let slept = time::ticks_to_seconds(b - a, tps);
    assert!((0.015..1.0).contains(&slept), "dormiu {slept} s");
    assert_eq!(time::ticks_to_seconds(500, 1000), 0.5);
    assert_eq!(time::ticks_to_seconds(1, 0), 0.0);
}

#[test]
fn utc_now_is_a_plausible_date() {
    let t = time::utc_now();
    assert!(t.year >= 2024, "{t:?}");
    assert!((1..=12).contains(&t.month) && (1..=31).contains(&t.day), "{t:?}");
    assert!(t.hour < 24 && t.minute < 60 && t.second < 61, "{t:?}");
}

#[test]
fn file_writer_writes_and_create_dir_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("forja_os_contract_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir_str = dir.to_str().unwrap();
    assert!(file::create_dir(dir_str));
    assert!(file::create_dir(dir_str), "pasta existente também é sucesso");

    let path = dir.join("saida.txt");
    let f = FileWriter::create(path.to_str().unwrap()).unwrap();
    assert!(f.write_all(b"forja "));
    assert!(f.write_all("ação\n".as_bytes()));
    f.close();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "forja ação\n");

    // sobrescreve
    let f = FileWriter::create(path.to_str().unwrap()).unwrap();
    assert!(f.write_all(b"2"));
    f.close();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "2");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_paths_are_rejected() {
    assert!(FileWriter::create("").is_none());
    assert!(FileWriter::create("a\0b").is_none());
    assert!(FileWriter::create(&"x".repeat(PATH_MAX + 1)).is_none());
    assert!(!file::create_dir(""));
}

#[test]
fn stack_capture_resolves_to_this_test_binary() {
    let mut frames = [0usize; 8];
    let n = stack::capture(0, &mut frames);
    assert!(n > 0);
    let mut buf = [0u8; 256];
    let mut t = TextBuf::new(&mut buf);
    stack::write_module_offset(&mut t, frames[0]);
    assert!(t.as_str().starts_with("contract"), "{}", t.as_str());
    assert!(t.as_str().contains("+0x"), "{}", t.as_str());
}

#[test]
fn thread_ids_differ_between_threads() {
    let here = process::thread_id();
    let there = std::thread::spawn(process::thread_id).join().unwrap();
    assert_ne!(here, 0);
    assert_ne!(here, there);
}

#[test]
fn memory_reports_are_plausible() {
    let s = process::system_memory().unwrap();
    assert!(s.total_physical > 0 && s.available_physical <= s.total_physical, "{s:?}");
    let p = process::process_memory().unwrap();
    assert!(p.resident > 0 && p.peak_resident >= p.resident / 2, "{p:?}");
}
