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
  frame, crash report). Nada de apelidos locais.
- **Comentários, docs e mensagens de log em português.**
- Tipos em `PascalCase`, funções e métodos em `snake_case`, constantes em `UPPER_CASE`.
- Pastas: `engine/`, `games/`, `examples/`, `tests/`, `tools/`, `docs/`. Dentro de `engine/`,
  pastas com nome curto (`engine/mem`) e crates com prefixo (`forja_mem`): o prefixo evita
  colisão com o `core` do Rust e com crates externos.

## Plataformas suportadas

A lista oficial é o `enum Os` em `engine/os/src/lib.rs` (`forja_os::CURRENT` diz para qual o
build foi compilado). Um SO fora dela para no `compile_error!` do `forja_os`, o único do
projeto.

| | Windows | Linux |
|---|---|---|
| Alvo | `x86_64-pc-windows-gnu` | `x86_64-unknown-linux-gnu`, Ubuntu desktop nativo |
| Testado em | só verificado (`cargo clippy --target x86_64-pc-windows-gnu`, sem gerar `.exe`)¹ | WSL2 (Ubuntu + WSLg) e Xvfb; Ubuntu nativo pendente |
| Memória (região, páginas) | `VirtualAlloc` | `mmap` |
| Relógio e sleep | `QueryPerformanceCounter`, `Sleep` | `clock_gettime`, `nanosleep` |
| Arquivos | Win32 (`CreateFileW`) | `open`/`write` |
| Bibliotecas dinâmicas | `LoadLibraryW` | `dlopen` |
| Erro fatal | filtro de exceções (SEH) | sinais + pilha alternativa |
| Pilha e módulo | `RtlCaptureStackBackTrace` | unwinder da libgcc + `dladdr` |
| Build id | GUID+age da entrada CodeView do PE | nota `NT_GNU_BUILD_ID` do ELF |
| Janela e input | Win32 | X11 (carregada em tempo de execução); em sessão Wayland, via XWayland |
| Modo headless | sim | sim |

¹ Desde 2026-10-30 o desenvolvimento roda no Linux (WSL): o antivírus corporativo bloqueia
executáveis recompilados no Windows. O código Windows continua sendo escrito junto (todo
contrato tem os dois backends) e verificado pelo clippy, mas não é executado até haver
liberação; o que foi escrito sem rodar está marcado no código.

Para adicionar uma plataforma, siga o roteiro no comentário de `forja_os::Os`: variante e
`CURRENT`, backend `engine/os/src/<so>.rs` (os testes de contrato dizem se cumpre),
dependências no `Cargo.toml`, backend de janela em `engine/platform` e esta tabela.

O alvo final do Linux é o Ubuntu desktop nativo; o WSL2 é o ambiente de desenvolvimento. O
Ubuntu usa Wayland por padrão (sem sessão Xorg no GNOME desde a 25.10), e a janela X11 roda
pelo XWayland, que vem em toda instalação desktop, o mesmo caminho do WSLg. Um backend Wayland
nativo (escala fracionada sem borrão, HDR) fica para depois, com o X11 de fallback: no GNOME o
próprio app desenha a barra de título (`libdecor`), o que custa bem mais que o X11.

## Camada de SO

- **Só o `forja_os` fala com o sistema operacional** (e o backend de janela do
  `forja_platform`). Nenhum outro crate depende de `windows-sys` ou `libc`.
- O **contrato** é o `engine/os/src/lib.rs`: tipos e funções públicas, documentados com unidades,
  erros, thread-safety e "não aloca". Os backends (`windows.rs`, `linux.rs`) são escolhidos em
  tempo de compilação: sem trait, sem dispatch. Backend fora do contrato não compila.
- O que difere de verdade entre sistemas fica escrito no contrato (ex.: granularidade de
  reserva é 64 KB no Windows e uma página no Linux). Código e testes não assumem valores do
  Windows.
- Os **testes de contrato** (`engine/os/tests/contract.rs`) e os crash tests valem para os dois
  sistemas; hoje rodam no WSL, e o Windows é verificado pelo clippy (ver "Validação").
- `forja_platform` segue o mesmo modelo para a janela: Win32 no Windows, X11 no Linux. A
  `libX11.so.6` é carregada em tempo de execução (`forja_os::library`), então não precisa do
  pacote de desenvolvimento para compilar. Em tempo de execução precisa do pacote `libx11-6`
  (todo desktop tem): sem ele, `init` devolve `DisplayLibraryMissing`; sem servidor gráfico,
  `DisplayUnavailable`.
- Modo `--headless N` do jogo (dt fixo, entrada roteirizada): depuração, profiling e máquinas
  sem servidor gráfico.

## Desenvolvimento no WSL

- Edita-se no `D:` (Windows); o WSL enxerga o mesmo projeto em `/mnt/d/...`.
- No WSL, os artefatos ficam no disco do Linux para não misturar com o `target` do Windows e
  compilar mais rápido: `export CARGO_TARGET_DIR=$HOME/.cache/forja-target`.
- Quebras de linha: o `.gitattributes` fixa LF para todo texto (scripts `.sh` quebram com CRLF).
- Limpeza: `tools/clean.sh` apaga os artefatos do cargo (`$CARGO_TARGET_DIR` e `target/`), os
  crash reports, os traces, `dist/` e `symbols/`. `-n` só lista; `-y` não pergunta. Nunca apaga
  um `$FORJA_SYMBOLS` fora do projeto.

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
- **Newtypes para ids e índices** (`Zone(u16)`, `Counter(u16)`, `Key(u8)`): não dá para
  passar um pelo outro.
- **Campos privados quando há invariante** (`Arena`, `TextBuf`); `pub` quando é só dado.

## Memória

- **Uma chamada ao SO.** `MemoryRegion::reserve` faz um único `VirtualAlloc` com o orçamento
  total do jogo. Tudo sai dali.
- **Arenas recortadas da região** (`region.arena(bytes)`): o jogo cria as que precisar, por
  tempo de vida (ex.: `persistent`, que nunca zera, e `frame`, que zera todo frame com
  `reset`). Arenas são `Send` mas nunca `Sync`: podem mudar de thread, nunca ser
  compartilhadas. O compilador garante.
- **Arena é só o cursor**, dentro dela (`Cell`): sem atômicos, locks ou registro global. A API
  é `alloc_bytes`/`alloc_slice`/`alloc_value`, `reset`, `used` e `capacity`.
- **Heap geral só conta** (`forja_mem::global`, o `#[global_allocator]`): atende o que não
  controlamos (std, dependências), repassa ao heap do sistema e conta alocações e bytes vivos
  (`global::stats()`). A meta do jogo é **0 alocações por frame** nele (contador no trace).
  Relatório com pilha, tags ou orçamento só voltam quando houver um caso real que precise.
- **Containers da std não são usados** no runtime da engine e do jogo. No lugar:
  - listas e tabelas: `arena.alloc_slice` (capacidade fixa decidida na criação)
  - texto: `TextBuf` sobre um buffer da stack ou da arena + `write!`
  - log: `log::info(format_args!(...))`
  - arquivos: `forja_os::file::FileWriter` (a `std::fs` aloca no Windows)
- **Páginas**: a região é arredondada para o bloco de reserva do SO (64 KB) e cada arena começa
  numa página (4 KB) com tamanho múltiplo de página; os bytes do arredondamento ficam com a
  própria arena. Dentro da arena não há alinhamento a página: os dados ficam colados.
- **Falta de espaço devolve `None`**: quem chama decide. Na inicialização, sair com erro; no
  meio do jogo, quando for fatal, `crash::fatal("OOM de arena", ...)` com o pedido e o uso
  da arena nos detalhes (vão para o stderr, não para o relatório).
- **Stack**: 1 MB na thread principal do Windows. Buffers grandes vão para arena.

## Diagnóstico

- `forja_diag::log`: `log::info/warn/error(format_args!(...))` em stderr, sem alocar.
- `crash::install` no início do `main`: panic, exceções do SO (access violation, stack
  overflow...) e erros fatais da engine geram `crash/crash_AAAA-MM-DD_HHMMSS.txt`.
- **O crash report é o mesmo em desenvolvimento e em produção, e só tem campos fixos**: jogo e
  versão, SO, data, tempo rodando, frame, motivo, código do SO, tipo de acesso (endereço nulo
  ou inválido), pilha em módulo+deslocamento e memória do sistema. Como no Shipping da Unreal,
  nada de texto montado em tempo de execução (mensagem de panic, detalhes do erro, log) nem
  endereço absoluto: o arquivo pode sair da máquina do jogador sem levar dados dele. O texto
  livre vai só para o stderr (`[crash] motivo: detalhes`).
- Todo caminho que aborta de propósito chama `crash::fatal` antes (o `abort` do Windows pula
  os handlers de exceção).
- Perfis com `panic = "abort"`: o hook grava o relatório e encerra; não há unwind.
- Pilha simbolizada: `tools/symbolize.sh <relatório-ou-log> [exe]` (addr2line). Ver
  "Release e símbolos".

## Release e símbolos

O jogo grava só o que existe em qualquer build (módulo+deslocamento e o build id); os nomes
vêm depois, fora do jogo, com o binário completo guardado. É o modelo da Unreal (PDBs fora
do pacote), do Breakpad/Crashpad e do Sentry. Nada de simbolizar dentro do processo: aloca,
pega lock e precisaria dos símbolos na máquina do jogador.

```
tools/release.sh racing     (no WSL)
  ├─ binário completo ──► $FORJA_SYMBOLS/racing/<build-id>/   privado; nunca distribuir
  └─ strip ───────────► dist/racing-<versão>-linux/            o que o jogador recebe
crash do jogador ──► crash_*.txt (build: <id>) ──► tools/symbolize.sh crash_*.txt
```

- **Build id**: identificador gravado pelo linker (`--build-id`, ligado no `.cargo/config.toml`
  para dev e release), lido por `forja_os::stack::build_id()` no `crash::install`. Vai na
  linha `build:` do relatório e no log (`crash report ativo; build: ...`). Sobrevive ao strip.
- **Perfil release**: `debug = "line-tables-only"` (arquivo e linha, sem tipos nem variáveis).
  Quem tira os símbolos é o script, depois de guardá-los.
- **`tools/release.sh <pacote>`**: compila, guarda o binário completo e um manifesto (versão,
  commit, alvo, data, rustc) em `$FORJA_SYMBOLS/<pacote>/<build-id>/` (padrão `symbols/`),
  gera a cópia sem símbolos em `dist/` e confere: mesmo build id, sem DWARF nem tabela de
  símbolos no `dist/`, nenhum caminho da máquina de build nos dois binários.
- **Caminhos remapeados** (`--remap-path-prefix`): sem isso, caminhos absolutos com o nome do
  usuário ficam no binário (mensagens de panic, DWARF), mesmo depois do strip.
- **`tools/symbolize.sh <relatório>`**: acha o binário pelo build id do arquivo. Com o
  executável explícito (build de dev), confere o build id antes: binário errado é erro, porque
  símbolos errados parecem certos. O resultado sai na tela e é salvo ao lado da entrada como
  `.symbol.txt` (`crash/crash_X.txt` → `crash/crash_X.symbol.txt`).
- **Guarde o `symbols/` de toda versão publicada** fora do repositório (`symbols/` e `dist/`
  estão no `.gitignore`). Sem o binário completo, os crash reports daquela versão não são
  legíveis.
- Fica de fora de propósito: minidump (copia memória do processo, pode levar dados do
  jogador), envio automático (exige consentimento, LGPD, e servidor), assinatura e CI.
- Windows: o `forja_os` já lê o build id do PE, mas o release Windows (MinGW no WSL) fica
  para quando o `.exe` puder ser executado.

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

Depois de cada mudança, no WSL (antes de uma versão, também no Ubuntu nativo):

```
cargo test --workspace
cargo clippy --workspace --all-targets
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu
```

A última linha verifica o código Windows sem gerar executável (precisa de
`rustup target add x86_64-pc-windows-gnu` no WSL, uma vez).
