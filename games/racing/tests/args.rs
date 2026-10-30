//! Argumentos de linha de comando do racing: argumento que não é UTF-8 válido não derruba o
//! jogo (`std::env::args` entraria em pânico com ele; usamos `args_os`).

// Arquivo só de teste: as mesmas exceções do clippy.toml valem para as funções auxiliares.
#![allow(clippy::unwrap_used)]

use std::ffi::OsString;
use std::process::Command;

fn invalid_utf8_arg() -> OsString {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        // "a" + surrogate UTF-16 solto: válido no Windows, impossível em UTF-8.
        OsString::from_wide(&[0x61, 0xD800])
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        // "a" + byte 0xFF: válido no Linux, impossível em UTF-8.
        OsString::from_vec(vec![0x61, 0xFF])
    }
}

/// Roda o racing headless numa pasta temporária (o trace é gravado na pasta atual).
fn run_headless(args: &[OsString]) -> (Option<i32>, String) {
    let dir = std::env::temp_dir().join(format!("forja_racing_args_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_racing")).args(args).current_dir(&dir).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (output.status.code(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn invalid_utf8_arguments_are_ignored() {
    // Argumento inválido solto, antes do --headless: ignorado.
    let (code, stderr) = run_headless(&[invalid_utf8_arg(), "--headless".into(), "3".into()]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stderr.contains("modo headless: 3 frames"), "{stderr}");
    assert!(!stderr.contains("[crash]"), "{stderr}");

    // N inválido: vira o padrão, sem pânico.
    let (code, stderr) = run_headless(&["--headless".into(), invalid_utf8_arg()]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stderr.contains("modo headless: 600 frames"), "{stderr}");
}
