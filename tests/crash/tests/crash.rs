//! Roda cada modo de crash_tests como processo separado e confere o relatório gravado.

// Arquivo só de teste: as mesmas exceções do clippy.toml valem para as funções auxiliares.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Outcome {
    code: Option<i32>,
    stderr: String,
    report: String,
}

fn run(mode: &str) -> Outcome {
    let dir: PathBuf = std::env::temp_dir().join(format!("forja_crash_{mode}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_crash_tests")).arg(mode).arg(&dir).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let files: Vec<PathBuf> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(files.len(), 1, "esperava um relatório em {dir:?}; stderr:\n{stderr}");
    let report = fs::read_to_string(&files[0]).unwrap();
    let _ = fs::remove_dir_all(&dir);
    Outcome { code: output.status.code(), stderr, report }
}

fn check_common(o: &Outcome, reason: &str) {
    assert_ne!(o.code, Some(0), "o processo deveria ter falhado");
    assert!(o.report.starts_with("=== Forja: relatório de crash ==="), "{}", o.report);
    assert!(o.report.contains(&format!("motivo:  {reason}\n")), "{}", o.report);
    assert!(o.report.contains("jogo:    crash_tests"), "{}", o.report);
    assert!(o.report.contains("frame:   42"), "{}", o.report);
    assert!(o.report.contains("crash_tests.exe+0x"), "pilha sem o módulo:\n{}", o.report);
    assert!(o.report.contains("--- memória do sistema ---"), "{}", o.report);
    assert!(o.report.contains("--- memória da forja ---"), "{}", o.report);
    assert!(o.report.contains("crash_tests: modo"), "breadcrumbs sem o log:\n{}", o.report);
    assert!(o.stderr.contains("[crash] relatório em"), "{}", o.stderr);
}

#[test]
fn arena_oom() {
    let o = run("oom");
    check_common(&o, "OOM de arena");
    assert!(o.report.contains("arena 'frame' sem espaço: pedido de 1000 bytes"), "{}", o.report);
    assert!(o.report.contains("(último pedido que falhou: 1000 bytes)"), "{}", o.report);
    assert!(o.report.contains("passou de 90% do orçamento"), "{}", o.report);
}

#[test]
fn allocation_with_locked_global() {
    let o = run("locked");
    check_common(&o, "alocação com o alocador global travado");
    assert!(o.report.contains("pedido de 100 bytes"), "{}", o.report);
}

#[test]
fn global_allocator_exhausted() {
    let o = run("exhausted");
    check_common(&o, "alocador global esgotado");
}

#[test]
fn panic_with_message_and_location() {
    let o = run("panic");
    check_common(&o, "panic");
    assert!(o.report.contains("teste de panic: índice 7"), "{}", o.report);
    assert!(o.report.contains("main.rs:"), "{}", o.report);
}

#[test]
fn access_violation() {
    let o = run("access");
    check_common(&o, "access violation");
    assert!(o.report.contains("tentou escrever o endereço 0x0000000000000000"), "{}", o.report);
    assert!(o.report.contains("exceção 0x"), "{}", o.report);
}

#[test]
fn stack_overflow() {
    let o = run("stack");
    check_common(&o, "stack overflow");
}
