# Estilo de código da Forja

Rust direto, sem custo escondido: qualquer pessoa que leia C deve entender o código, e tudo que
ele faz deve estar à vista.

## Princípios

1. **A engine é uma biblioteca.** O jogo é dono do `main`, do loop e do layout dos dados.
   A engine nunca chama o jogo.
2. **Nada é obrigatório.** Cada módulo é um crate separado; quem não usa, não compila e não paga.
3. **Sem estado global.** Todo estado vive em structs que o jogo possui e passa por parâmetro.
   Exceções documentadas: log, crash report e alocador global (são do processo inteiro).
4. **Memória decidida por quem chama.** Módulos não crescem buffers por conta própria: a
   capacidade é definida na criação e documentada.
5. **Estados inválidos não existem.** O tipo impede o uso errado em vez de checar depois.
6. **Ferramentas de desenvolvimento somem no build final** (features desligáveis, `cfg!` constante).

## Idioma e nomes

- **Identificadores em inglês**, com a terminologia padrão da indústria e do Rust (arena,
  frame, breadcrumbs, crash report). Nada de apelidos locais.
- **Comentários, docs e mensagens de log em português.**
- Tipos em `PascalCase`, funções e métodos em `snake_case`, constantes em `UPPER_CASE`.
- Pastas: `engine/`, `games/`, `tests/`, `tools/`, `docs/`. Dentro de `engine/`, pastas com
  nome curto (`engine/mem`) e crates com prefixo (`forja_mem`): o prefixo evita colisão com o
  `core` do Rust e com crates externos.

## Camada de SO

- **Só o `forja_os` fala com o sistema operacional** (e o backend de janela do
  `forja_platform`). Nenhum outro crate depende de `windows-sys` ou `libc`.
- O **contrato** é o `engine/os/src/lib.rs`: tipos e funções públicas, documentados com unidades,
  erros, thread-safety e "não aloca". Os backends (`windows.rs`, `linux.rs`) são escolhidos em
  tempo de compilação: sem trait, sem dispatch. Backend fora do contrato não compila.
- O que difere de verdade entre sistemas fica escrito no contrato (ex.: granularidade de
  reserva é 64 KB no Windows e uma página no Linux). Código e testes não assumem valores do
  Windows.
- Os **testes de contrato** (`engine/os/tests/contract.rs`) e os crash tests rodam nos dois
  sistemas. Validação completa = Windows **e** WSL.
- `forja_platform` segue o mesmo modelo para a janela: Win32 no Windows, X11 no Linux. A
  `libX11.so.6` é carregada em tempo de execução (`forja_os::library`), então não precisa do
  pacote de desenvolvimento para compilar; sem servidor gráfico, `init` devolve
  `DisplayUnavailable`. Outros sistemas: erro de compilação.
- Modo `--headless N` do jogo (dt fixo, entrada roteirizada): depuração, profiling e máquinas
  sem servidor gráfico.

## Desenvolvimento no WSL

- Edita-se no `D:` (Windows); o WSL enxerga o mesmo projeto em `/mnt/d/...`.
- No WSL, os artefatos ficam no disco do Linux para não misturar com o `target` do Windows e
  compilar mais rápido: `export CARGO_TARGET_DIR=$HOME/.cache/forja-target`.
- Quebras de linha: o `.gitattributes` fixa LF para todo texto (scripts `.sh` quebram com CRLF).

## Tipos e funções

- **Operações de um tipo são métodos** (`impl Tipo`): `arena.alloc_slice(n, 0)`,
  `platform.poll()`, `prof.begin(ZONE_FRAME)`. Um método é só outra sintaxe para uma função:
  mesmo código gerado, sem dispatch dinâmico.
- **Operações sem tipo dono são funções do módulo**, chamadas com o módulo na frente:
  `log::info(...)`, `crash::fatal(...)`, `global::lock()`, `math::clamp(...)`.
- **Sistemas do jogo continuam funções livres** que processam lotes de dados
  (`cars_update(&mut table, dt)`): orientação a dados, não a objetos.
- **Construtores validam** e devolvem `Option`/`Result`: `MemoryRegion::reserve`,
  `Platform::init`, `FileWriter::create`. Se o valor existe, ele é válido.
- **Consumir para encerrar**: `platform.shutdown()`, `file.close()` tomam `self`; usar depois
  não compila.
- **Enums no lugar de constantes numéricas** (`EventKind`, `MouseButton`), `#[repr(u8)]` quando
  o layout importa.
- **Newtypes para ids e índices** (`Zone(u16)`, `Counter(u16)`, `Key(u8)`, `Mark`): não dá para
  passar um pelo outro.
- **Campos privados quando há invariante** (`Arena`, `TextBuf`); `pub` quando é só dado.

## Memória

- **Uma chamada ao SO.** `MemoryRegion::reserve` faz um único `VirtualAlloc` com o orçamento
  total do jogo. Tudo sai dali.
- **Cada thread tem a sua `ThreadMemory`**: arenas `persistent` (programa inteiro), `level`
  (zera ao trocar de fase/pista) e `frame` (zera todo frame). Arenas são `Send` mas nunca
  `Sync`: podem mudar de thread, nunca ser compartilhadas. O compilador garante.
- **Heap geral monitorado** (`forja_mem::global`, o `#[global_allocator]`), no modelo da Unreal
  (FMalloc + LLM + Memory Insights). Atende só o que não controlamos (std, dependências):
  - backend: heap do sistema (`HeapAlloc`/LFH), trocável sem mudar quem usa;
  - contabilidade: bytes vivos, pico e alocações, por tag (`AllocTag`, por thread);
  - política: `Startup` (aloca calado), `Report` (depois de `global::end_startup()`: cada ponto
    de chamada que alocar vai para o log uma vez, com a pilha), `Forbid` (fatal; testes e CI);
  - orçamento (`global::set_budget`): passar dele avisa no log;
  - OOM do sistema e liberação com tamanho errado (heap corrompido) geram crash report;
  - em debug, memória nova sai com `0xCD` e liberada com `0xDD`.
  - A meta do jogo é **0 alocações por frame** no heap geral (contador no trace).
- **Containers da std não são usados** no runtime da engine e do jogo. No lugar:
  - listas e tabelas: `arena.alloc_slice` (capacidade fixa decidida na criação)
  - texto: `TextBuf` sobre um buffer da stack ou da arena + `write!`
  - log: `log::info(format_args!(...))`
  - arquivos: `forja_os::file::FileWriter` (a `std::fs` aloca no Windows)
- **Páginas**: a região é arredondada para o bloco de reserva do SO (64 KB) e cada arena começa
  numa página (4 KB) com tamanho múltiplo de página; os bytes do arredondamento ficam com a
  própria arena. Dentro da arena não há alinhamento a página: os dados ficam colados.
- **Orçamento visível**: ao passar de 90%, a arena avisa no log (uma vez). Falta de espaço
  devolve `None`; quando for fatal, `arena.oom_fatal()` grava o crash report com o pedido que
  falhou e o uso de todas as arenas.
- **Stack**: 1 MB na thread principal do Windows. Buffers grandes vão para arena.

## Diagnóstico

- `forja_diag::log`: `log::info/warn/error(format_args!(...))`, sem alocar; as últimas linhas
  ficam num ring buffer (breadcrumbs) que vai para o crash report.
- `crash::install` no início do `main`: panic, exceções do SO (access violation, stack
  overflow...) e erros fatais da engine geram `crash/crash_AAAA-MM-DD_HHMMSS.txt` com motivo,
  pilha, memória do sistema, memória da forja, seções registradas e breadcrumbs.
- Todo caminho que aborta de propósito chama `crash::fatal` antes (o `abort` do Windows pula
  os handlers de exceção).
- Perfis com `panic = "abort"`: o hook grava o relatório e encerra; não há unwind.
- Pilha simbolizada: `tools/symbolize.sh <relatório-ou-log> <exe>` (addr2line, mesmo build);
  serve também para os avisos `heap geral: ... pilha: ...` do log.

## Usa / não usa

| Usa | Não usa |
|---|---|
| `struct` com campos `pub` (ou privados com invariante) | traits próprias, `impl Trait`, `dyn` |
| métodos `impl Tipo` e funções de módulo | genéricos (exceto `alloc_slice`/`alloc_value`) e macros próprias |
| enums, newtypes, construtores que validam | constantes numéricas soltas para estados e ids |
| arenas, `TextBuf`, buffers na stack | `Vec`, `String`, `Box`, `format!`, `std::fs` |
| slices `&[T]`, iteradores simples (`iter`, `zip`) | `Rc`, `RefCell`, `Box<dyn>`, closures guardadas |
| `let Some(x) = ... else { return ... }`, `get()` | `unwrap`, `expect`, indexação `a[i]`, slicing `a[i..j]` |
| `checked_*`, `wrapping_*`, `saturating_*` em inteiros | `+ - * / %` em inteiros (pode estourar ou dividir por zero) |
| nomes como `plus`/`minus`/`scaled` em vetores | sobrecarga de operadores (`a + b` em vetores) |
| erros como enum simples ou `Option` | `panic!`, `unreachable!`, `todo!` |

Traits da std só quando a linguagem exige: `fmt::Write` (para `write!`), `GlobalAlloc`.
Derives básicos (`Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Default`) são aceitos.
Aritmética de `f32`/`f64` é livre (não gera panic).

As regras de panic são impostas por lints `deny` em `Cargo.toml`; testes podem usar `unwrap` e
indexação (`clippy.toml`).

## Validação

Depois de cada mudança, no Windows e no WSL:

```
cargo test --workspace
cargo clippy --workspace --all-targets
```
