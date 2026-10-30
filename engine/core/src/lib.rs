//! Base da engine: tipos e utilitarios sem nenhum contato com o SO.
//!
//! Nao depende de nada alem de `core`. Qualquer crate da engine pode
//! depender daqui; este crate nao depende de nenhum outro.

#![no_std]

/// Trava tamanho e alinhamento de um tipo em tempo de compilacao.
///
/// Usado em toda struct `#[repr(C)]` que cruza uma fronteira (FFI com o SO,
/// contrato entre camadas). Layout errado e UB silencioso; aqui vira erro de
/// compilacao.
///
/// `forge_core::layout!(WNDCLASSEXW, size = 80, align = 8);`
#[macro_export]
macro_rules! layout {
    ($t:ty, size = $size:expr, align = $align:expr) => {
        const _: () = {
            assert!(::core::mem::size_of::<$t>() == $size, concat!("size_of<", stringify!($t), "> diverge do esperado"));
            assert!(::core::mem::align_of::<$t>() == $align, concat!("align_of<", stringify!($t), "> diverge do esperado"));
        };
    };
}
