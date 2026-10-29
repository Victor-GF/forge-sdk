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

## Usa / não usa

| Usa | Não usa |
|---|---|
| `struct` `#[repr(C)]` com campos `pub` | traits próprias, `impl Trait`, `dyn` |
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
