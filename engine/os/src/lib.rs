//! Camada de sistema operacional (OS abstraction layer), como a HAL da Unreal.
//!
//! **Este arquivo é o contrato.** Cada função pública documenta unidades, erros, se é
//! thread-safe e que não aloca; o comportamento é o mesmo em todos os sistemas suportados, e o
//! que difere de verdade (ex.: granularidade de reserva) está escrito aqui.
//!
//! Os backends (`windows.rs`, `linux.rs`) implementam as funções de `sys` com as APIs de cada
//! SO e são escolhidos em tempo de compilação: sem dispatch, sem custo. Um backend que faltar
//! uma função ou mudar uma assinatura não compila.
//!
//! Regras do projeto:
//! - só este crate (e o backend de janela do `forja_platform`) depende de `windows-sys`/`libc`;
//! - os testes de contrato (`tests/contract.rs`) rodam iguais em todos os sistemas.
//!
//! Nada aqui aloca no heap do Rust: tudo pode ser chamado com o heap geral em
//! `HeapPolicy::Forbid`. (O carregador de bibliotecas do SO usa memória própria.)

// ---------------------------------------------------------------- plataformas suportadas
//
// **Fonte única da lista de plataformas da Forja.** Todo crate da engine depende deste, então
// um SO fora da lista para aqui, com a mensagem abaixo, antes de qualquer outro erro.
//
// Para adicionar uma plataforma (ex.: macOS):
// 1. aqui: variante em `Os`, `CURRENT`, condição do `compile_error!` e `path` do `mod sys`;
// 2. `engine/os/src/<so>.rs`: todas as funções de `sys` (os testes de contrato dizem se cumpre);
// 3. `engine/os/Cargo.toml`: dependências daquele SO;
// 4. `engine/platform`: `path` do backend de janela e `src/<so>.rs`;
// 5. a tabela "Plataformas suportadas" em `docs/style.md`.

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("Forja: plataforma não suportada. Suportadas: Windows e Linux (ver forja_os::Os).");

/// Sistemas operacionais suportados pela Forja.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
}

impl Os {
    pub fn name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Linux => "linux",
        }
    }
}

/// O sistema para o qual este build foi compilado.
#[cfg(windows)]
pub const CURRENT: Os = Os::Windows;
/// O sistema para o qual este build foi compilado.
#[cfg(target_os = "linux")]
pub const CURRENT: Os = Os::Linux;

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
mod sys;

// ---------------------------------------------------------------- memória

pub mod memory {
    use crate::sys;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PageInfo {
        /// Tamanho da página em bytes (potência de 2; 4096 na maioria das máquinas x86_64).
        pub page_size: usize,
        /// Granularidade de `reserve`: o endereço devolvido é múltiplo dela.
        /// 64 KB no Windows; igual à página no Linux.
        pub allocation_granularity: usize,
    }

    /// Thread-safe.
    pub fn page_info() -> PageInfo {
        sys::page_info()
    }

    /// Pede memória nova ao SO: `size` bytes legíveis e graváveis, zerados, começando num
    /// múltiplo de `allocation_granularity` (o SO arredonda `size` para cima).
    ///
    /// Nunca é devolvida ao SO: o ponteiro vale até o processo acabar. Páginas nunca tocadas
    /// não ocupam RAM física (no Windows contam no limite de commit; no Linux, não).
    ///
    /// `None` se `size == 0` ou o SO recusar. Thread-safe.
    pub fn reserve(size: usize) -> Option<*mut u8> {
        if size == 0 {
            return None;
        }
        sys::memory_reserve(size)
    }
}

// ---------------------------------------------------------------- tempo

pub mod time {
    use crate::sys;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UtcTime {
        pub year: u16,
        /// 1 a 12.
        pub month: u8,
        /// 1 a 31.
        pub day: u8,
        pub hour: u8,
        pub minute: u8,
        pub second: u8,
    }

    /// Relógio monotônico de alta resolução: nunca volta. Unidade: ticks
    /// (`ticks_per_second` por segundo). Thread-safe.
    pub fn ticks() -> u64 {
        sys::ticks()
    }

    /// Ticks por segundo do relógio de `ticks` (sempre > 0). Thread-safe.
    pub fn ticks_per_second() -> u64 {
        sys::ticks_per_second().max(1)
    }

    /// Converte ticks em segundos. `ticks_per_second == 0` dá 0.
    pub fn ticks_to_seconds(ticks: u64, ticks_per_second: u64) -> f64 {
        if ticks_per_second == 0 {
            return 0.0;
        }
        ticks as f64 / ticks_per_second as f64
    }

    /// Dorme pelo menos `ms` milissegundos. Com `begin_precise_sleep` ativo, a precisão fica
    /// perto de 1 ms nos dois sistemas.
    pub fn sleep_ms(ms: u32) {
        sys::sleep_ms(ms);
    }

    /// Pede ao SO sleeps com precisão de ~1 ms (Windows: `timeBeginPeriod`; Linux: já é
    /// preciso, não faz nada). Pareie com `end_precise_sleep`.
    pub fn begin_precise_sleep() {
        sys::begin_precise_sleep();
    }

    pub fn end_precise_sleep() {
        sys::end_precise_sleep();
    }

    /// Data e hora atuais em UTC. Pode ser chamada dentro do handler de erro fatal.
    pub fn utc_now() -> UtcTime {
        sys::utc_now()
    }
}

// ---------------------------------------------------------------- arquivos

pub mod file {
    use crate::sys;

    /// Maior caminho aceito, em bytes UTF-8. O SO pode ter um limite menor (no Windows, 260
    /// caracteres sem o modo de caminhos longos).
    pub const PATH_MAX: usize = 1024;

    /// Arquivo aberto para escrita. Só existe aberto: `create` devolve `None` se falhar e
    /// `close` consome o valor.
    pub struct FileWriter {
        handle: sys::FileHandle,
    }

    fn path_ok(path: &str) -> bool {
        !path.is_empty() && path.len() <= PATH_MAX && !path.contains('\0')
    }

    impl FileWriter {
        /// Cria (ou sobrescreve) o arquivo. `None` se o caminho for vazio, maior que
        /// `PATH_MAX`, tiver byte zero, ou o SO recusar. Não aloca.
        pub fn create(path: &str) -> Option<FileWriter> {
            if !path_ok(path) {
                return None;
            }
            sys::file_create(path).map(|handle| FileWriter { handle })
        }

        /// Escreve tudo. `false` se o SO falhar no meio.
        pub fn write_all(&self, data: &[u8]) -> bool {
            sys::file_write_all(&self.handle, data)
        }

        pub fn close(self) {
            sys::file_close(self.handle);
        }
    }

    /// Cria uma pasta (um nível). `true` se criou ou se ela já existia.
    pub fn create_dir(path: &str) -> bool {
        path_ok(path) && sys::create_dir(path)
    }
}

// ---------------------------------------------------------------- pilha

pub mod stack {
    use forja_core::text::TextBuf;

    use crate::sys;

    pub const FRAMES_MAX: usize = 62;

    /// Preenche `frames` com os endereços de retorno da pilha atual, do mais recente para o mais
    /// antigo, pulando `skip` quadros além de quem chamou. Devolve quantos foram capturados.
    /// Não aloca; pode ser chamada dentro do handler de erro fatal.
    #[inline(always)]
    pub fn capture(skip: u32, frames: &mut [usize]) -> usize {
        sys::stack_capture(skip, frames)
    }

    /// Escreve `módulo+0xdeslocamento` (`racing.exe+0x1a2b` no Windows, `racing+0x1a2b` no
    /// Linux): o deslocamento é relativo ao início do módulo e pode ser simbolizado com
    /// `tools/symbolize.sh`. Endereço fora de módulo: não escreve nada e devolve `false` (quem
    /// chama decide se mostra o endereço cru). Não aloca.
    pub fn write_module_offset(out: &mut TextBuf, address: usize) -> bool {
        sys::write_module_offset(out, address)
    }
}

// ---------------------------------------------------------------- erro fatal

pub mod fatal {
    use crate::sys;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MemoryAccess {
        Read,
        Write,
        Execute,
        Unknown,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FatalKind {
        AccessViolation { address: usize, access: MemoryAccess },
        StackOverflow,
        IllegalInstruction,
        DivideByZero,
        IntegerOverflow,
        /// Erro de barramento / página que não pôde ser lida (ex.: arquivo mapeado sumiu).
        BusError,
        Breakpoint,
        Other,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FatalEvent {
        pub kind: FatalKind,
        /// Código bruto do SO: NTSTATUS no Windows, número do sinal no Linux.
        pub os_code: u32,
        /// Instrução que falhou.
        pub instruction_address: usize,
    }

    /// Instala `handler` para erros fatais vindos do hardware/SO: acesso inválido, stack
    /// overflow, instrução ilegal, divisão por zero...
    ///
    /// Contrato do handler:
    /// - roda na thread que falhou; na thread que chamou `install` há pelo menos 32 KB de
    ///   stack, inclusive em stack overflow (nas outras, a reserva padrão do Rust);
    /// - não pode alocar nem esperar locks;
    /// - quando ele volta, o processo termina com falha.
    ///
    /// `panic!` e `abort()` não passam por aqui: use o hook de panic da std e grave o relatório
    /// antes de abortar. Chamar de novo troca o handler.
    pub fn install(handler: fn(&FatalEvent)) {
        sys::fatal_install(handler);
    }

    /// Encerra o processo na hora, sem destrutores nem handlers. No Linux só os 8 bits baixos
    /// de `exit_code` chegam a quem esperava o processo.
    pub fn terminate(exit_code: u32) -> ! {
        sys::terminate(exit_code)
    }
}

// ---------------------------------------------------------------- bibliotecas dinâmicas

pub mod library {
    use core::ffi::c_void;

    use crate::sys;

    /// Maior nome aceito (de biblioteca ou de símbolo), em bytes UTF-8.
    pub const NAME_MAX: usize = 255;

    /// Biblioteca dinâmica carregada (`.dll` no Windows, `.so` no Linux). Só existe carregada:
    /// `open` devolve `None` se falhar e `close` consome o valor.
    ///
    /// O carregador do SO usa memória própria (não o heap geral do Rust).
    pub struct Library {
        handle: sys::LibraryHandle,
    }

    fn name_ok(name: &str) -> bool {
        !name.is_empty() && name.len() <= NAME_MAX && !name.contains('\0')
    }

    impl Library {
        /// Carrega pelo nome do arquivo (`"kernel32.dll"`, `"libX11.so.6"`), procurando nos
        /// caminhos padrão do SO. `None` se não existir ou não puder ser carregada.
        pub fn open(name: &str) -> Option<Library> {
            if !name_ok(name) {
                return None;
            }
            sys::library_open(name).map(|handle| Library { handle })
        }

        /// Endereço do símbolo exportado `name`, ou `None`. Quem chama converte para o tipo de
        /// função certo (`transmute`), e é responsável por a assinatura bater com a da biblioteca.
        pub fn symbol(&self, name: &str) -> Option<*mut c_void> {
            if !name_ok(name) {
                return None;
            }
            sys::library_symbol(&self.handle, name)
        }

        /// Descarrega. Ponteiros obtidos com `symbol` deixam de valer.
        pub fn close(self) {
            sys::library_close(self.handle);
        }
    }
}

// ---------------------------------------------------------------- processo

pub mod process {
    use crate::sys;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SystemMemory {
        pub total_physical: u64,
        /// No Linux é aproximado (livre + buffers).
        pub available_physical: u64,
        /// Limite de commit do sistema (RAM + arquivo de paginação). `None` no Linux.
        pub commit_limit: Option<u64>,
        pub commit_available: Option<u64>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ProcessMemory {
        /// Memória física em uso (working set / RSS).
        pub resident: u64,
        pub peak_resident: u64,
        /// Memória commitada pelo processo. `None` no Linux.
        pub committed: Option<u64>,
        pub peak_committed: Option<u64>,
    }

    /// Id da thread atual no SO (diferente entre threads vivas).
    pub fn thread_id() -> u64 {
        sys::thread_id()
    }

    /// Memória da máquina. `None` se o SO não responder. Não aloca.
    pub fn system_memory() -> Option<SystemMemory> {
        sys::system_memory()
    }

    /// Memória deste processo. `None` se o SO não responder. Não aloca.
    pub fn process_memory() -> Option<ProcessMemory> {
        sys::process_memory()
    }
}
