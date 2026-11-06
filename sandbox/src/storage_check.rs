//! Testa o armazenamento portavel uma vez na inicializacao: conteudo e
//! saves por nome, sem nenhum caminho. E a parte de arquivos que um console
//! tambem teria.
//!
//! Deixa a pasta de saves do sandbox criada (vazia) em `%APPDATA%`.

use forge_os::prelude::*;

const GAME: &str = "forge-sandbox";
const SAVE: &str = "slot-1.sav";

/// `content` e um arquivo que existe no conteudo do jogo.
pub fn run(content: &str) {
    let file = Platform::content_open(content).expect("content_open falhou");
    let size = Platform::file_size(&file).expect("file_size falhou");
    Platform::file_close(file);
    assert!(Platform::content_open("nao-existe.bin").is_none(), "conteudo inexistente abriu");

    let saves = Platform::save_mount(GAME).expect("save_mount falhou");
    let data: Vec<u8> = (0..10_000u32).map(|i| (i * 13 + i / 7) as u8).collect();
    assert!(Platform::save_write(&saves, SAVE, &data));
    // Sobrescrever troca o save inteiro: o tamanho e o do novo.
    assert!(Platform::save_write(&saves, SAVE, &data[..5_000]));
    assert_eq!(Platform::save_size(&saves, SAVE), Some(5_000));

    let mut buf = vec![0u8; 8_000];
    assert_eq!(Platform::save_read(&saves, SAVE, &mut buf), Some(5_000));
    assert!(buf[..5_000] == data[..5_000], "save lido diverge do gravado");

    assert!(!Platform::save_write(&saves, "../fora.sav", &data), "nome com diretorio foi aceito");
    assert!(Platform::save_delete(&saves, SAVE));
    assert!(Platform::save_size(&saves, SAVE).is_none(), "save apagado ainda existe");
    Platform::save_unmount(saves);

    forge_log!("[storage] ok: conteudo {content} ({size} bytes), save gravado, sobrescrito, lido e apagado");
}
