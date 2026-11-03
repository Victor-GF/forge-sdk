# forge-sdk

Engine no estilo Handmade Hero, em Rust. Dependencia minima de abstracoes
ocultas: nenhum crate externo e bibliotecas `no_std`. Tudo que fala com o SO
e escrito a mao.

Alvo: Windows x86_64. O backend Linux esta desabilitado ate definirmos a
API grafica.

## Estrutura

```
forge-sdk/
├── engine/
│   ├── core/       forge_core     no_std. Tipos e utilitarios base, sem SO.
│   └── os/         forge_os       Contrato `Os` + backends por plataforma.
│       └── src/
│           ├── lib.rs             `Os`, `Platform`, `prelude`
│           ├── process.rs         categoria: trait `Process`
│           ├── memory.rs          categoria: trait `Memory`
│           ├── thread.rs          categoria: trait `Thread`
│           ├── sync.rs            categoria: trait `Synchronization`
│           ├── window.rs          categoria: trait `Window`
│           ├── dialog.rs          categoria: trait `Dialog`
│           ├── linux/             desabilitado (mod.rs vazio, nao compilado)
│           └── win32/             um arquivo por categoria (impl das traits)
│               └── ffi/           bindings a mao, um arquivo por header do SDK
└── sandbox/        forge_sandbox  Binario `sandbox`: app de teste que usa a engine.
```

Novos modulos da engine entram como `engine/<nome>` e sao incluidos no
workspace automaticamente.

## Regras de dependencia

```
sandbox ──> os ──> core
```

- `sandbox` fica fora de `engine/` e consome a engine como um jogo
  consumiria. Nada da engine depende dele.
- `core` nao depende de nada.
- `os` e o unico crate que toca o SO. Todo `extern "system"` vive em
  `os/src/win32/ffi/`.
- Quem usa a engine nao chama o SO diretamente: usa `forge_os::Platform`.

## Contrato com o SO

O contrato e dividido por categoria de funcionalidade do SO. Cada categoria
e uma trait com funcoes associadas, no seu modulo:

| Categoria | Trait | Funcoes |
|---|---|---|
| `process` | `Process` | `exit_process` |
| `memory` | `Memory` | paginas e heap (abaixo) |
| `thread` | `Thread` | `thread_create`, `thread_join`, `thread_detach`, `thread_current_id`, `thread_sleep`, `thread_yield`, `logical_processor_count` |
| `sync` | `Synchronization` | `semaphore_create`, `semaphore_wait`, `semaphore_signal`, `semaphore_destroy` |
| `window` | `Window` | `window_create`, `window_destroy`, `window_pump_events`, `window_close_requested`, `window_size` |
| `dialog` | `Dialog` | `message_box` |

`forge_os::Os` junta todas as categorias.
Cada backend implementa todas num tipo concreto, um arquivo por categoria
(`win32/memory.rs`, ...), e `forge_os::Platform` aponta para o backend do
alvo via `cfg`. A dispatch e estatica: `Platform::message_box(...)` vira uma
chamada direta, sem `dyn` nem vtable.

As funcoes pertencem as traits de categoria, que precisam estar em escopo:

```rust
use forge_os::prelude::*;

Platform::message_box(c"forge", c"Forge SDK");
```

Para adicionar algo a uma categoria: declarar na trait dela e implementar em
cada backend. Para uma categoria nova: criar `src/<categoria>.rs` com a trait,
incluir a trait em `Os` e no `prelude`, e implementar em
`<backend>/<categoria>.rs`. Um backend incompleto nao compila.

### Memoria

Duas formas, ambas direto do SO, sem alocador intermediario:

| | Funcoes | Win32 | Para que |
|---|---|---|---|
| Paginas | `page_info`, `pages_reserve`, `pages_commit`, `pages_alloc`, `pages_decommit`, `pages_release` | `VirtualAlloc` / `VirtualFree` | blocos grandes e de vida longa: memoria do jogo, arenas |
| Heap | `heap_alloc`, `heap_alloc_zeroed`, `heap_realloc`, `heap_free` | `HeapAlloc` / `HeapReAlloc` / `HeapFree` no heap do processo | blocos pequenos e de vida variada (alinhamento 16) |

Falha volta como ponteiro nulo (ou `false` em `pages_commit`); quem chama
decide o que fazer. Funcoes que recebem um ponteiro sao `unsafe`.

### Handles

Threads, semaforos e janelas sao devolvidos como handles opacos
(`ThreadHandle`, `SemaphoreHandle`, `WindowHandle`), com `None` se a criacao
falhar. A funcao que libera consome o handle, entao usar depois de liberar
nao compila.

- `ThreadHandle` e `Send`: pode ser esperado de outra thread.
- `SemaphoreHandle` e `Send + Sync`: compartilhado por referencia.
- `WindowHandle` nao e `Send`: a janela pertence a thread que a criou.

## no_std

As bibliotecas da engine (`core`, `os`) sao `no_std`. Tudo que fala com o SO
passa por `forge_os`, que chama as DLLs do sistema direto.

## Regras de interface com o SO

A camada de SO nao tem testes automatizados: e critica e e verificada a mao,
declaracao por declaracao, contra a fonte oficial.

- Linkagem `kind = "raw-dylib"`; declaracoes em `ffi/` seguem o nome e
  o valor do Windows SDK.
- `ffi/` tem um arquivo por header do SDK, com o mesmo nome
  (`memoryapi.h` → `ffi/memoryapi.rs`). Cada simbolo fica no arquivo do
  header que o declara; `ffi/mod.rs` reexporta tudo.
- Toda struct `#[repr(C)]` que cruza fronteira leva
  `forge_core::layout!(T, size = N, align = M)` com os valores x86_64. Essa
  checagem roda em tempo de compilacao e continua valendo.

## Build

```
cargo build              engine + sandbox
cargo run -p forge_sandbox
```
