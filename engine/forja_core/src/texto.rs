//! Texto num buffer fornecido por quem chama (stack ou arena), sem alocar.
//! Implementa `core::fmt::Write` para funcionar com `write!`. O que não couber é cortado
//! (sempre numa fronteira de caractere) e `truncou` fica `true`.

use core::fmt;

pub struct Texto<'a> {
    buf: &'a mut [u8],
    len: usize,
    pub truncou: bool,
}

pub fn texto_novo(buf: &mut [u8]) -> Texto<'_> {
    Texto { buf, len: 0, truncou: false }
}

pub fn texto_str<'b>(t: &'b Texto) -> &'b str {
    core::str::from_utf8(texto_bytes(t)).unwrap_or("")
}

pub fn texto_bytes<'b>(t: &'b Texto) -> &'b [u8] {
    t.buf.get(..t.len).unwrap_or(&[])
}

pub fn texto_limpar(t: &mut Texto) {
    t.len = 0;
    t.truncou = false;
}

// `impl` de trait da std, necessária para `write!`.
impl fmt::Write for Texto<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let livre = self.buf.len().saturating_sub(self.len);
        let mut n = s.len().min(livre);
        while n > 0 && !s.is_char_boundary(n) {
            n = n.saturating_sub(1);
        }
        if n < s.len() {
            self.truncou = true;
        }
        let fim = self.len.saturating_add(n);
        if let (Some(origem), Some(destino)) = (s.as_bytes().get(..n), self.buf.get_mut(self.len..fim)) {
            destino.copy_from_slice(origem);
            self.len = fim;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    #[test]
    fn formata_sem_alocar() {
        let mut buf = [0u8; 32];
        let mut t = texto_novo(&mut buf);
        write!(t, "{} ms | {:.1}", 16, 2.31f32).unwrap();
        assert_eq!(texto_str(&t), "16 ms | 2.3");
        assert!(!t.truncou);
        texto_limpar(&mut t);
        assert_eq!(texto_str(&t), "");
    }

    #[test]
    fn corta_em_fronteira_de_caractere() {
        let mut buf = [0u8; 4];
        let mut t = texto_novo(&mut buf);
        write!(t, "aaçç").unwrap(); // "aa" + "ç" (2 bytes) cabe; o segundo "ç" não
        assert_eq!(texto_str(&t), "aaç");
        assert!(t.truncou);
    }
}
