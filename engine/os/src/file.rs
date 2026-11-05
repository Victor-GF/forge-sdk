//! Arquivos: ler, escrever e mapear arquivos, listar diretorios e achar os
//! caminhos do jogo.
//!
//! # Caminhos
//!
//! Caminhos sao `&str` UTF-8 com `/` como separador em todas as plataformas
//! (o Windows aceita `/`). Caminhos devolvidos pelo SO tambem vem com `/`.
//!
//! Caminho relativo e resolvido a partir do diretorio de trabalho, que
//! depende de quem abriu o jogo (atalho, terminal, debugger). Para assets,
//! monte o caminho a partir de [`FileSystem::path_executable`].
//!
//! Sem alocacao: o que o SO devolve e escrito num buffer de quem chama.
//! [`PATH_CAPACITY`] bytes bastam na pratica; caminho que nao cabe falha.
//!
//! # Leitura e escrita
//!
//! Sempre com `offset` explicito: o handle nao tem cursor. Por isso varias
//! threads podem ler o mesmo arquivo ao mesmo tempo, cada uma na sua faixa
//! (jobs do pool carregando partes de um pacote de assets).
//!
//! Abrir nao trava o arquivo para outros processos, como no Linux: uma
//! ferramenta pode substituir um asset enquanto o jogo esta com ele aberto.

use core::ffi::c_void;
use core::slice;

/// Tamanho de buffer, em bytes, para caminhos. Caminho de entrada precisa ser
/// menor que isso; caminho de saida pode precisar de mais se tiver muitos
/// caracteres fora do ASCII.
pub const PATH_CAPACITY: usize = 1024;

/// Arquivo aberto por `file_open`. Pode ser lido e escrito de varias threads
/// ao mesmo tempo (`*_at`). Precisa terminar em `file_close`.
pub struct FileHandle(pub(crate) *mut c_void);

unsafe impl Send for FileHandle {}
unsafe impl Sync for FileHandle {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAccess {
    /// So leitura. O arquivo precisa existir.
    Read,
    /// So escrita. Cria o arquivo, ou apaga o conteudo se ja existe.
    Write,
}

/// Tipo de uma entrada de `directory_list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

/// Arquivo inteiro mapeado na memoria, so leitura, por `file_map`. Continua
/// valido depois de `file_close`; precisa terminar em `file_unmap`.
pub struct FileView {
    pub(crate) ptr: *const u8,
    pub(crate) len: usize,
}

unsafe impl Send for FileView {}
unsafe impl Sync for FileView {}

impl FileView {
    /// O conteudo do arquivo. Se outro processo mudar o arquivo enquanto ele
    /// esta mapeado, o conteudo muda junto; se o arquivo encolher, ler alem
    /// do novo fim derruba o processo. Mapeie so arquivos que o jogo controla.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.ptr, self.len) }
    }
}

/// Tudo antes do ultimo `/`: o diretorio de um caminho. Vazio se nao houver
/// `/`.
pub fn path_parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

pub trait FileSystem {
    // ---- Arquivos ---------------------------------------------------------

    /// Abre `path` para `access`. `None` se falhar (nao existe, sem
    /// permissao, caminho invalido ou longo demais).
    fn file_open(path: &str, access: FileAccess) -> Option<FileHandle>;

    fn file_close(file: FileHandle);

    /// Tamanho atual do arquivo, em bytes. `None` se falhar.
    fn file_size(file: &FileHandle) -> Option<u64>;

    /// Le a partir de `offset` ate encher `buf` ou o arquivo acabar, e
    /// devolve quantos bytes leu: menos que `buf.len()` so no fim do arquivo,
    /// zero se `offset` ja passou do fim. `None` se falhar.
    fn file_read_at(file: &FileHandle, offset: u64, buf: &mut [u8]) -> Option<usize>;

    /// Escreve `data` inteiro a partir de `offset`; passar do fim aumenta o
    /// arquivo. `false` se falhar, e entao parte pode ter sido escrita.
    fn file_write_at(file: &FileHandle, offset: u64, data: &[u8]) -> bool;

    /// Mapeia o arquivo inteiro, so leitura, sem copia: as paginas sao lidas
    /// do disco quando acessadas. O arquivo precisa ter sido aberto com
    /// `Read`. `None` se falhar ou se o arquivo estiver vazio.
    fn file_map(file: &FileHandle) -> Option<FileView>;

    fn file_unmap(view: FileView);

    /// Apaga o arquivo. `false` se falhar.
    fn file_delete(path: &str) -> bool;

    /// Renomeia ou move `from` para `to`, substituindo `to` se existir. No
    /// mesmo volume, a troca e atomica: para salvar sem risco de deixar um
    /// arquivo pela metade se o jogo cair, escreva num temporario e renomeie
    /// por cima do definitivo. `false` se falhar.
    fn file_rename(from: &str, to: &str) -> bool;

    /// Copia `from` para `to`, substituindo `to` se existir. `false` se falhar.
    fn file_copy(from: &str, to: &str) -> bool;

    /// Momento da ultima escrita no arquivo, no relogio de arquivos do SO
    /// (nao e o de `time`). So serve para comparar com outro valor do mesmo
    /// arquivo: se mudou, o arquivo foi reescrito. `None` se falhar.
    fn file_modified(path: &str) -> Option<u64>;

    // ---- Diretorios -------------------------------------------------------

    /// Cria o diretorio; o pai precisa existir. `true` se criou ou se ja
    /// existia.
    fn directory_create(path: &str) -> bool;

    /// Apaga o diretorio, que precisa estar vazio. `false` se falhar.
    fn directory_delete(path: &str) -> bool;

    /// Chama `visit` com o nome (sem o caminho) e o tipo de cada entrada de
    /// `path`, sem `.` e `..`, em ordem qualquer. Nomes que nao sao Unicode
    /// valido (possivel no Windows e no Linux) sao pulados: nao dariam para
    /// reabrir com `&str`. `false` se o diretorio nao abriu.
    fn directory_list(path: &str, visit: impl FnMut(&str, EntryKind)) -> bool;

    // ---- Caminhos do jogo -------------------------------------------------

    /// Caminho completo do executavel, escrito em `out`. O diretorio dele
    /// ([`path_parent`]) e a base para os assets. `None` se nao couber.
    fn path_executable(out: &mut [u8]) -> Option<&str>;

    /// Diretorio do usuario para dados de aplicativos, escrito em `out`:
    /// saves e configuracoes vao numa subpasta com o nome do jogo, criada
    /// com `directory_create`. `None` se falhar ou nao couber.
    fn path_user_data(out: &mut [u8]) -> Option<&str>;
}
