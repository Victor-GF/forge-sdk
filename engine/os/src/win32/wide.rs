//! Conversao entre UTF-8 e UTF-16 na stack, para as APIs `...W` do Windows.
//!
//! Sem alocacao. Texto de log, de tamanho livre, sai em pedacos
//! (`for_each_chunk`); um pedaco nunca separa um par surrogate, entao cada um
//! e UTF-16 valido sozinho. Caminhos precisam ir inteiros (`exact`): o que
//! nao cabe falha.

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

/// Escreve `text` em UTF-16 no comeco de `out`, sem nulo final, e devolve
/// quantas unidades escreveu. `None` se nao couber ou se `text` tiver um nulo,
/// que cortaria o texto na API.
pub(super) fn encode(text: &str, out: &mut [u16]) -> Option<usize> {
    let mut len = 0;
    for unit in text.encode_utf16() {
        if unit == 0 || len == out.len() {
            return None;
        }
        out[len] = unit;
        len += 1;
    }
    Some(len)
}

/// `text` inteiro em UTF-16, terminado em nulo. `None` se nao couber em `N`
/// unidades, contando o nulo, ou se tiver um nulo.
pub(super) fn exact<const N: usize>(text: &str) -> Option<[u16; N]> {
    let mut buf = [0u16; N];
    encode(text, &mut buf[..N - 1])?;
    Some(buf)
}

/// Converte `units` para UTF-8 no comeco de `out` e devolve quantos bytes
/// escreveu, sempre caracteres inteiros. `None` se nao couber ou se `units`
/// nao for UTF-16 valido (o Windows aceita surrogate solto em nome de
/// arquivo).
pub(super) fn decode(units: &[u16], out: &mut [u8]) -> Option<usize> {
    let mut len = 0;
    for c in char::decode_utf16(units.iter().copied()) {
        let c = c.ok()?;
        let size = c.len_utf8();
        if len + size > out.len() {
            return None;
        }
        c.encode_utf8(&mut out[len..len + size]);
        len += size;
    }
    Some(len)
}
