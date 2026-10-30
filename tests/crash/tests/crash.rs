//! Roda cada modo de crash_tests como processo separado e confere o relatório gravado.

// Arquivo só de teste: as mesmas exceções do clippy.toml valem para as funções auxiliares.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Como o módulo aparece na pilha (`forja_os::stack::write_module_offset`).
const MODULE: &str = if cfg!(windows) { "crash_tests.exe+0x" } else { "crash_tests+0x" };

/// Linux: o build id do relatório é o do binário em disco, lido por uma ferramenta independente
/// (`readelf`, binutils): confere o leitor do `forja_os` contra o que o linker gravou.
#[cfg(target_os = "linux")]
fn build_id_matches(build: &str) -> bool {
    let out = Command::new("readelf").arg("-n").arg(env!("CARGO_BIN_EXE_crash_tests")).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let id = text.lines().find_map(|l| l.trim().strip_prefix("Build ID: ")).unwrap_or_default();
    assert!(!id.is_empty(), "readelf não achou build id:\n{text}");
    build == id
}

/// Windows: GUID + age da entrada CodeView (20 bytes).
#[cfg(windows)]
fn build_id_matches(build: &str) -> bool {
    build.len() == 40 && build.bytes().all(|c| c.is_ascii_hexdigit())
}

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

/// Campos fixos presentes; nada de texto livre (detalhe, log, arquivo do código) nem endereço
/// absoluto no arquivo.
fn check_common(o: &Outcome, reason: &str) {
    assert_ne!(o.code, Some(0), "o processo deveria ter falhado");
    assert!(o.report.starts_with("=== Forja: relatório de crash ==="), "{}", o.report);
    assert!(o.report.contains(&format!("motivo:  {reason}\n")), "{}", o.report);
    assert!(o.report.contains("jogo:    crash_tests"), "{}", o.report);
    assert!(o.report.contains("frame:   42"), "{}", o.report);
    assert!(o.report.contains(&format!("sistema: {}\n", std::env::consts::OS)), "{}", o.report);
    let build = o.report.lines().find_map(|l| l.strip_prefix("build:   ")).unwrap_or("");
    assert!(build_id_matches(build), "build id errado:\n{}", o.report);
    assert!(o.report.contains(MODULE), "pilha sem o módulo:\n{}", o.report);
    assert!(o.report.contains("--- memória do sistema ---"), "{}", o.report);
    for forbidden in ["detalhe", "crash_tests: modo", ".rs:"] {
        assert!(!o.report.contains(forbidden), "'{forbidden}' no relatório:\n{}", o.report);
    }
    let absolute = o.report.split_whitespace().find(|w| w.starts_with("0x") && w.len() == 18);
    assert_eq!(absolute, None, "endereço absoluto no relatório:\n{}", o.report);
    assert!(o.stderr.contains(&format!("[crash] {reason}")), "{}", o.stderr);
    assert!(o.stderr.contains("[crash] relatório em"), "{}", o.stderr);
}

#[test]
fn arena_oom() {
    let o = run("oom");
    check_common(&o, "OOM de arena");
    assert!(o.stderr.contains("arena 'frame' sem espaço: pedido de 1000 bytes"), "{}", o.stderr);
    assert!(o.stderr.contains("passou de 90% do orçamento"), "{}", o.stderr);
    assert!(!o.report.contains("arena 'frame'"), "{}", o.report);
}

#[test]
fn allocation_with_forbidden_heap() {
    let o = run("forbid");
    check_common(&o, "alocação com o heap geral proibido");
    assert!(o.stderr.contains("pedido de 100 bytes"), "{}", o.stderr);
    assert!(o.stderr.contains("HeapPolicy::Forbid"), "{}", o.stderr);
}

#[test]
fn heap_out_of_memory() {
    let o = run("heap_oom");
    check_common(&o, "heap geral sem memória");
    assert!(o.stderr.contains("o sistema recusou 70368744177664 bytes"), "{}", o.stderr);
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
fn panic_message_goes_only_to_stderr() {
    let o = run("panic");
    check_common(&o, "panic");
    assert!(o.stderr.contains("teste de panic: índice 7"), "{}", o.stderr);
    assert!(o.stderr.contains("main.rs:"), "{}", o.stderr);
    assert!(!o.report.contains("teste de panic"), "{}", o.report);
}

#[test]
fn access_violation() {
    let o = run("access");
    check_common(&o, "access violation");
    assert!(o.report.contains("erro SO: 0x"), "{}", o.report);
    assert!(o.report.contains("acesso:  escrever em endereço nulo"), "{}", o.report);
    assert!(o.report.contains(&format!("exceção {MODULE}")), "{}", o.report);
}

#[test]
fn stack_overflow() {
    let o = run("stack");
    check_common(&o, "stack overflow");
}

/// Argumento que não é UTF-8 válido (`std::env::args` entraria em pânico com ele).
fn invalid_utf8_arg() -> std::ffi::OsString {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        // "a" + surrogate UTF-16 solto: válido no Windows, impossível em UTF-8.
        std::ffi::OsString::from_wide(&[0x61, 0xD800])
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        // "a" + byte 0xFF: válido no Linux, impossível em UTF-8.
        std::ffi::OsString::from_vec(vec![0x61, 0xFF])
    }
}

#[test]
fn invalid_utf8_argument_is_an_error_not_a_crash() {
    for args in [vec![invalid_utf8_arg()], vec!["report".into(), invalid_utf8_arg()]] {
        let output = Command::new(env!("CARGO_BIN_EXE_crash_tests")).args(&args).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains("não é UTF-8 válido"), "{stderr}");
        assert!(!stderr.contains("[crash]"), "não deveria haver crash: {stderr}");
    }
}
