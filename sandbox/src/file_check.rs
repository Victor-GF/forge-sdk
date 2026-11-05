//! Testa a categoria de arquivos uma vez na inicializacao, num diretorio
//! temporario ao lado do executavel: caminhos, salvar com rename, leitura
//! posicional, mapeamento, listagem e limpeza.
//!
//! O nome do arquivo tem acentos de proposito: exercita a conversao UTF-8 /
//! UTF-16 nos dois sentidos (abrir pelo nome e ler o nome na listagem).

use forge_os::file::{PATH_CAPACITY, path_parent};
use forge_os::prelude::*;

const SIZE: usize = 100_000;

pub fn run() {
    let mut exe_buf = [0u8; PATH_CAPACITY];
    let exe = Platform::path_executable(&mut exe_buf).expect("path_executable falhou");
    forge_log!("[file] executavel: {exe}");

    let mut user_buf = [0u8; PATH_CAPACITY];
    match Platform::path_user_data(&mut user_buf) {
        Some(user) => forge_log!("[file] dados do usuario: {user}"),
        None => forge_log!("[file] sem diretorio de dados do usuario"),
    }

    let dir = format!("{}/file_check", path_parent(exe));
    assert!(Platform::directory_create(&dir), "directory_create falhou");
    assert!(Platform::directory_create(&dir), "criar diretorio que ja existe deveria dar certo");

    let content: Vec<u8> = (0..SIZE as u32).map(|i| (i * 7 + i / 251) as u8).collect();
    let temp = format!("{dir}/salvando.tmp");
    let path = format!("{dir}/coração.bin");

    // Salvar sem risco: escreve no temporario, em duas partes, e renomeia por
    // cima do definitivo.
    let file = Platform::file_open(&temp, FileAccess::Write).expect("file_open Write falhou");
    let (first, second) = content.split_at(SIZE / 2);
    assert!(Platform::file_write_at(&file, 0, first));
    assert!(Platform::file_write_at(&file, first.len() as u64, second));
    Platform::file_close(file);
    assert!(Platform::file_rename(&temp, &path), "file_rename falhou");
    assert!(Platform::file_open(&temp, FileAccess::Read).is_none(), "temporario deveria ter sumido");

    let file = Platform::file_open(&path, FileAccess::Read).expect("file_open Read falhou");
    assert_eq!(Platform::file_size(&file), Some(SIZE as u64));

    // Perto do fim le menos que o buffer; depois do fim, zero.
    let mut buf = [0u8; 1000];
    let tail = Platform::file_read_at(&file, (SIZE - 300) as u64, &mut buf).expect("file_read_at falhou");
    assert_eq!(&buf[..tail], &content[SIZE - 300..]);
    assert_eq!(Platform::file_read_at(&file, SIZE as u64 + 10, &mut buf), Some(0));

    // A view continua valida depois de fechar o arquivo.
    let view = Platform::file_map(&file).expect("file_map falhou");
    Platform::file_close(file);
    assert!(view.bytes() == content.as_slice(), "conteudo mapeado diverge");
    Platform::file_unmap(view);

    let mut entries = Vec::new();
    assert!(Platform::directory_list(&dir, |name, kind| entries.push((name.to_string(), kind))));
    assert_eq!(entries, [("coração.bin".to_string(), EntryKind::File)]);

    assert!(Platform::file_delete(&path), "file_delete falhou");
    assert!(Platform::directory_delete(&dir), "directory_delete falhou");
    assert!(!Platform::directory_list(&dir, |_, _| {}), "diretorio deveria ter sumido");

    forge_log!("[file] ok: {SIZE} bytes salvos, lidos, mapeados e apagados");
}
