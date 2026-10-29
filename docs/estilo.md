# Estilo de código da Forja

Rust usado como "C com segurança". O objetivo é que qualquer pessoa que leia C entenda o código,
e que nenhum custo fique escondido.

## Princípios

1. **A engine é uma biblioteca.** O jogo é dono do `main`, do loop e do layout dos dados.
   A engine nunca chama o jogo.
2. **Nada é obrigatório.** Cada módulo é um crate separado; quem não usa, não compila e não paga.
3. **Sem estado global.** Todo estado vive em structs que o jogo possui e passa por parâmetro.
4. **Memória decidida por quem chama.** Módulos não crescem buffers por conta própria: a capacidade
   é definida na criação e documentada.
5. **Ferramentas de desenvolvimento somem no build final** (features desligáveis, `cfg!` constante).

## Memória

- **Uma chamada ao SO.** `memoria_init` faz um único `VirtualAlloc` com o orçamento total do
  jogo. Tudo sai dali.
- **Cada thread trabalha na sua mesa** (`Mesa`): arenas `persistente` (programa inteiro),
  `fase` (zera ao trocar de fase/pista) e `frame` (zera todo frame). Arenas são `Send` mas
  nunca `Sync`: podem mudar de thread, nunca ser compartilhadas. O compilador garante.
- **Malloc bloqueado.** O alocador global (`forja_mem::global`) só atende a inicialização da
  std, num buffer fixo. Depois de `global_travar()`, qualquer `Vec`, `String`, `Box`,
  `format!` (nosso ou de dependência) aborta o programa. No debugger, o abort mostra a pilha.
- **Containers da std não são usados** fora de testes. No lugar:
  - listas e tabelas: `arena_slice` (capacidade fixa decidida na criação)
  - texto: `Texto` sobre um buffer da stack ou da arena + `write!`
  - log: `log_info(format_args!(...))`
  - arquivos: `forja_platform::arquivo` (a `std::fs` aloca no Windows)
- **Stack**: 1 MB na thread principal do Windows. Buffers grandes vão para arena.
- **Exceção aos genéricos:** `arena_slice`/`arena_valor` (`T: Copy`), para entregar memória
  tipada sem `unsafe` no jogo.
- **Exceção ao estado global e às traits:** o alocador global (`unsafe impl GlobalAlloc`),
  exigido pela linguagem.

## Usa / não usa

| Usa | Não usa |
|---|---|
| `struct` `#[repr(C)]` com campos `pub` | traits próprias, `impl Trait`, `dyn` |
| arenas da mesa, `Texto`, buffers na stack | `Vec`, `String`, `Box`, `format!`, `std::fs` |
| funções livres `modulo_verbo(alvo, ...)` | genéricos e macros próprias |
| slices `&[T]`, iteradores simples (`iter`, `zip`) | `Rc`, `RefCell`, `Box<dyn>`, closures guardadas |
| `let Some(x) = ... else { return ... }`, `get()` | `unwrap`, `expect`, indexação `a[i]`, slicing `a[i..j]` |
| `checked_*`, `wrapping_*`, `saturating_*` em inteiros | `+ - * / %` em inteiros (pode estourar ou dividir por zero) |
| constantes e enums simples | sobrecarga de operadores (`a + b` em vetores) |
| erros como enum simples ou `Option` | `panic!`, `unreachable!`, `todo!` |

Derives básicos (`Debug`, `Clone`, `Copy`, `PartialEq`, `Default`) são aceitos.
Aritmética de `f32`/`f64` é livre (não gera panic).

Essas regras são impostas por lints `deny` em `Cargo.toml`; testes podem usar `unwrap` e indexação
(`clippy.toml`).

## Nomes

- Tipos em `PascalCase`, funções em `snake_case` com o prefixo do tipo ou módulo:
  `vec3_add`, `platform_poll`, `prof_inicio`.
- Constantes em `MAIUSCULAS`, com prefixo do grupo: `TECLA_ESC`, `ZONA_FRAME`.
- Português no código do jogo e da engine; nomes da API do sistema operacional ficam como são.

## Validação

Depois de cada mudança:

```
cargo test --workspace
cargo clippy --workspace --all-targets
```
