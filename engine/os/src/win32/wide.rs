//! UTF-8 para UTF-16 na stack, para as APIs `...W` do Windows.
//!
//! Sem alocacao: texto longo sai em pedacos. Um pedaco nunca separa um par
//! surrogate, entao cada um e UTF-16 valido sozinho.

/// Unidades UTF-16 por pedaco, contando o nulo final.
const CHUNK: usize = 512;

/// Chama `emit` com `text` em UTF-16, em pedacos terminados em nulo. O nulo
/// faz parte do slice: para APIs que pedem tamanho, use `len() - 1`.
pub(super) fn for_each_chunk(text: &str, mut emit: impl FnMut(&[u16])) {
    let mut buf = [0u16; CHUNK];
    let mut len = 0;
    for c in text.chars() {
        let mut units = [0u16; 2];
        let units = c.encode_utf16(&mut units);
        if len + units.len() + 1 > CHUNK {
            buf[len] = 0;
            emit(&buf[..=len]);
            len = 0;
        }
        buf[len..len + units.len()].copy_from_slice(units);
        len += units.len();
    }
    if len > 0 {
        buf[len] = 0;
        emit(&buf[..=len]);
    }
}

/// `text` em UTF-16 terminado em nulo, truncado para caber em `N` unidades.
pub(super) fn truncated<const N: usize>(text: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    let mut len = 0;
    for c in text.chars() {
        let mut units = [0u16; 2];
        let units = c.encode_utf16(&mut units);
        if len + units.len() + 1 > N {
            break;
        }
        buf[len..len + units.len()].copy_from_slice(units);
        len += units.len();
    }
    buf
}
