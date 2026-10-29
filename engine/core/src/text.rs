//! Texto num buffer fornecido por quem chama (stack ou arena), sem alocar.
//! Implementa `core::fmt::Write` para funcionar com `write!`. O que não couber é cortado
//! (sempre numa fronteira de caractere) e `truncated()` passa a devolver `true`.

use core::fmt;

pub struct TextBuf<'a> {
    buf: &'a mut [u8],
    len: usize,
    truncated: bool,
}

impl<'a> TextBuf<'a> {
    pub fn new(buf: &'a mut [u8]) -> TextBuf<'a> {
        TextBuf { buf, len: 0, truncated: false }
    }

    /// Sempre UTF-8 válido: só caracteres inteiros são copiados.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or(&[])
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.truncated = false;
    }
}

// `impl` de trait da std, necessária para `write!`.
impl fmt::Write for TextBuf<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let free = self.buf.len().saturating_sub(self.len);
        let mut n = s.len().min(free);
        while n > 0 && !s.is_char_boundary(n) {
            n = n.saturating_sub(1);
        }
        if n < s.len() {
            self.truncated = true;
        }
        let end = self.len.saturating_add(n);
        if let (Some(src), Some(dst)) = (s.as_bytes().get(..n), self.buf.get_mut(self.len..end)) {
            dst.copy_from_slice(src);
            self.len = end;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    #[test]
    fn formats_without_allocating() {
        let mut buf = [0u8; 32];
        let mut t = TextBuf::new(&mut buf);
        write!(t, "{} ms | {:.1}", 16, 2.31f32).unwrap();
        assert_eq!(t.as_str(), "16 ms | 2.3");
        assert!(!t.truncated());
        t.clear();
        assert_eq!(t.as_str(), "");
    }

    #[test]
    fn truncates_on_char_boundary() {
        let mut buf = [0u8; 4];
        let mut t = TextBuf::new(&mut buf);
        write!(t, "aaçç").unwrap(); // "aa" + "ç" (2 bytes) cabe; o segundo "ç" não
        assert_eq!(t.as_str(), "aaç");
        assert!(t.truncated());
    }
}
