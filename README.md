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
│   ├── os/         forge_os       Contrato `Os` + backends por plataforma.
│   │   └── src/
│   │       ├── linux/             desabilitado (mod.rs vazio, nao compilado)
│   │       └── win32/             ffi.rs (bindings a mao) + mod.rs (impl Os)
│   └── runtime/    forge_runtime  Binario `forge`. `fn main` padrao do Rust.
```

Novos modulos da engine entram como `engine/<nome>` e sao incluidos no
workspace automaticamente.

## Regras de dependencia

```
runtime ──> os ──> core
```

- `core` nao depende de nada.
- `os` e o unico crate que toca o SO. Todo `extern "system"` vive em
  `os/src/win32/ffi.rs`.
- `runtime` nao chama o SO diretamente: usa `forge_os::Platform`.

## Contrato com o SO

`forge_os::Os` e uma trait com funcoes associadas. Cada backend implementa a
trait num tipo concreto e `forge_os::Platform` aponta para o backend do alvo
via `cfg`. A dispatch e estatica: `Platform::message_box(...)` vira uma
chamada direta, sem `dyn` nem vtable.

Para adicionar algo ao contrato: declarar em `Os`, implementar em cada
backend. Um backend incompleto nao compila.

### Memoria

Duas formas, ambas direto do SO, sem alocador intermediario:

| | Funcoes | Win32 | Para que |
|---|---|---|---|
| Paginas | `page_info`, `pages_reserve`, `pages_commit`, `pages_alloc`, `pages_decommit`, `pages_release` | `VirtualAlloc` / `VirtualFree` | blocos grandes e de vida longa: memoria do jogo, arenas |
| Heap | `heap_alloc`, `heap_alloc_zeroed`, `heap_realloc`, `heap_free` | `HeapAlloc` / `HeapReAlloc` / `HeapFree` no heap do processo | blocos pequenos e de vida variada (alinhamento 16) |

Falha volta como ponteiro nulo (ou `false` em `pages_commit`); quem chama
decide o que fazer. Funcoes que recebem um ponteiro sao `unsafe`.

## no_std nas bibliotecas, main padrao no binario

As bibliotecas da engine (`core`, `os`) sao `no_std`. O runtime usa a
`fn main` padrao do Rust: a `std` cuida do startup, do panic e da saida, e o
cargo funciona com os padroes, sem configuracao de linker.

Como no Handmade Hero (que linka a CRT mas quase nao a usa), o runtime nao
usa a `std` para falar com o SO: tudo passa por `forge_os`, que chama as
DLLs do sistema direto.

## Regras de interface com o SO

A camada de SO nao tem testes automatizados: e critica e e verificada a mao,
declaracao por declaracao, contra a fonte oficial.

- Linkagem `kind = "raw-dylib"`; declaracoes em `ffi.rs` seguem o nome e
  o valor do Windows SDK.
- Toda struct `#[repr(C)]` que cruza fronteira leva
  `forge_core::layout!(T, size = N, align = M)` com os valores x86_64. Essa
  checagem roda em tempo de compilacao e continua valendo.

## Build

```
cargo run -p forge_runtime
```
