//! Roda cada modo de crash_testes como processo separado e confere o relatório gravado.

// Arquivo só de teste: as mesmas exceções do clippy.toml valem para as funções auxiliares.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Resultado {
    codigo: Option<i32>,
    stderr: String,
    relatorio: String,
}

fn rodar(modo: &str) -> Resultado {
    let pasta: PathBuf = std::env::temp_dir().join(format!("forja_crash_{modo}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).unwrap();
    let saida = Command::new(env!("CARGO_BIN_EXE_crash_testes"))
        .arg(modo)
        .arg(&pasta)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&saida.stderr).into_owned();
    let arquivos: Vec<PathBuf> = fs::read_dir(&pasta).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(arquivos.len(), 1, "esperava um relatório em {pasta:?}; stderr:\n{stderr}");
    let relatorio = fs::read_to_string(&arquivos[0]).unwrap();
    let _ = fs::remove_dir_all(&pasta);
    Resultado { codigo: saida.status.code(), stderr, relatorio }
}

fn conferir_comum(r: &Resultado, motivo: &str) {
    assert_ne!(r.codigo, Some(0), "o processo deveria ter falhado");
    assert!(r.relatorio.starts_with("=== Forja: relatório de crash ==="), "{}", r.relatorio);
    assert!(r.relatorio.contains(&format!("motivo:  {motivo}\n")), "{}", r.relatorio);
    assert!(r.relatorio.contains("jogo:    crash_testes"), "{}", r.relatorio);
    assert!(r.relatorio.contains("frame:   42"), "{}", r.relatorio);
    assert!(r.relatorio.contains("crash_testes.exe+0x"), "pilha sem o módulo:\n{}", r.relatorio);
    assert!(r.relatorio.contains("--- memória do sistema ---"), "{}", r.relatorio);
    assert!(r.relatorio.contains("--- memória da forja ---"), "{}", r.relatorio);
    assert!(r.relatorio.contains("crash_testes: modo"), "rastro sem o log:\n{}", r.relatorio);
    assert!(r.stderr.contains("[crash] relatório em"), "{}", r.stderr);
}

#[test]
fn oom_de_arena() {
    let r = rodar("oom");
    conferir_comum(&r, "OOM de arena");
    assert!(r.relatorio.contains("arena 'frame' sem espaço: pedido de 1000 bytes"), "{}", r.relatorio);
    assert!(r.relatorio.contains("(último pedido que falhou: 1000 bytes)"), "{}", r.relatorio);
    assert!(r.relatorio.contains("passou de 90% do orçamento"), "{}", r.relatorio);
}

#[test]
fn alocacao_com_global_travado() {
    let r = rodar("travado");
    conferir_comum(&r, "alocação com o alocador global travado");
    assert!(r.relatorio.contains("pedido de 100 bytes"), "{}", r.relatorio);
}

#[test]
fn alocador_global_esgotado() {
    let r = rodar("esgotado");
    conferir_comum(&r, "alocador global esgotado");
}

#[test]
fn panic_com_mensagem_e_local() {
    let r = rodar("panic");
    conferir_comum(&r, "panic");
    assert!(r.relatorio.contains("teste de panic: índice 7"), "{}", r.relatorio);
    assert!(r.relatorio.contains("main.rs:"), "{}", r.relatorio);
}

#[test]
fn access_violation() {
    let r = rodar("acesso");
    conferir_comum(&r, "access violation");
    assert!(r.relatorio.contains("tentou escrever o endereço 0x0000000000000000"), "{}", r.relatorio);
    assert!(r.relatorio.contains("exceção 0x"), "{}", r.relatorio);
}

#[test]
fn stack_overflow() {
    let r = rodar("pilha");
    conferir_comum(&r, "stack overflow");
}
