//! Roda cada modo de crash_tests como processo separado e confere o relatório gravado.

// Arquivo só de teste: as mesmas exceções do clippy.toml valem para as funções auxiliares.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Como o módulo aparece na pilha (`forja_os::stack::write_module_offset`).
const MODULE: &str = if cfg!(windows) { "crash_tests.exe+0x" } else { "crash_tests+0x" };

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
    assert!(o.report.contains(&format!("sistema: {}\n", std::env::consts::OS)), "{}", o.report);
    assert!(o.report.contains(MODULE), "pilha sem o módulo:\n{}", o.report);
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
fn allocation_with_forbidden_heap() {
    let o = run("forbid");
    check_common(&o, "alocação com o heap geral proibido");
    assert!(o.report.contains("pedido de 100 bytes"), "{}", o.report);
    assert!(o.report.contains("política Forbid"), "{}", o.report);
}

#[test]
fn heap_out_of_memory() {
    let o = run("heap_oom");
    check_common(&o, "heap geral sem memória");
    assert!(o.report.contains("o sistema recusou 70368744177664 bytes"), "{}", o.report);
}

#[test]
fn heap_reports_each_call_site_once_without_crashing() {
    let dir: PathBuf = std::env::temp_dir().join(format!("forja_crash_report_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_crash_tests")).arg("report").arg(&dir).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(!dir.exists(), "não deveria haver crash report");
    assert_eq!(stderr.matches("heap geral: alocação de").count(), 2, "{stderr}");
    assert!(stderr.contains("heap geral: alocação de 100 bytes"), "{stderr}");
    assert!(stderr.contains("heap geral: alocação de 2000 bytes"), "{stderr}");
    assert!(stderr.contains(&format!("pilha: {MODULE}")), "{stderr}");
    assert!(stderr.contains("heap geral passou do orçamento"), "{stderr}");
    assert!(stderr.contains("report: 2 pontos de chamada relatados"), "{stderr}");
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
