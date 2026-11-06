//! Armazenamento do jogo: conteudo (assets) e saves, sem caminhos.
//!
//! O jogo nao sabe onde os dados moram; cada plataforma decide:
//!
//! | | PC | Console |
//! |---|---|---|
//! | Conteudo | diretorio do executavel | pacote do jogo, montado so leitura |
//! | Saves | `%APPDATA%/<jogo>/` (Windows) | conteiner de save do sistema, por usuario |
//!
//! # Conteudo
//!
//! Somente leitura, por nome relativo com `/` (`texturas/grama.tex`). O
//! arquivo aberto e lido com [`Files`](crate::file::Files).
//!
//! # Saves
//!
//! Blobs inteiros por nome, numa area do jogo aberta com `save_mount`. Sem
//! escrita parcial: `save_write` grava tudo de uma vez, e e atomico. Se o
//! processo morrer no meio (queda de energia, crash), fica a versao anterior
//! inteira, nunca uma mistura.
//!
//! Nomes de save sao simples, para caber nas regras de qualquer plataforma:
//! letras, digitos, `_`, `-` e `.`, sem comecar com `.`, ate 64 bytes, sem
//! diretorios. Fora disso, a operacao falha.

use crate::file::FileHandle;
use core::ffi::c_void;

/// Area de saves aberta por `save_mount`. Precisa terminar em
/// `save_unmount`.
pub struct SaveHandle(pub(crate) *mut c_void);

/// Tamanho maximo de um nome de save, em bytes.
pub const SAVE_NAME_CAPACITY: usize = 64;

/// O nome segue as regras de save (ver o modulo).
pub fn save_name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= SAVE_NAME_CAPACITY
        && !name.starts_with('.')
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

pub trait Storage {
    /// Abre um arquivo de conteudo para leitura. `None` se nao existir.
    fn content_open(name: &str) -> Option<FileHandle>;

    /// Abre a area de saves do jogo `game`, criando se preciso. `game`
    /// segue as regras de nome de save. `None` se falhar.
    fn save_mount(game: &str) -> Option<SaveHandle>;

    fn save_unmount(saves: SaveHandle);

    /// Grava `data` como o save `name`, substituindo o anterior de forma
    /// atomica. So volta `true` depois que os dados estao no disco. `false`
    /// se falhar; o save anterior, se havia, continua intacto.
    fn save_write(saves: &SaveHandle, name: &str, data: &[u8]) -> bool;

    /// Tamanho do save, em bytes. `None` se nao existir.
    fn save_size(saves: &SaveHandle, name: &str) -> Option<u64>;

    /// Le o save a partir do comeco ate encher `buf` ou o save acabar, e
    /// devolve quantos bytes leu. `None` se nao existir ou falhar.
    fn save_read(saves: &SaveHandle, name: &str, buf: &mut [u8]) -> Option<usize>;

    /// Apaga o save. `false` se falhar ou nao existir.
    fn save_delete(saves: &SaveHandle, name: &str) -> bool;
}
