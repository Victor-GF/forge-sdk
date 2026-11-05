//! Cache: as duas distancias que guiam layout de dados.
//!
//! Aproveitar cache tem dois objetivos opostos, e quem escolhe e quem conhece
//! o padrao de acesso do tipo:
//!
//! - Densidade ([`CACHE_LINE`]): dados lidos juntos devem caber juntos. O
//!   padrao da engine: tipos pequenos, empacotados, SoA, campos quentes
//!   separados dos frios. Sem padding.
//! - Isolamento ([`CACHE_ISOLATION`], [`CacheAligned`]): dado escrito por uma
//!   thread enquanto outras leem ou escrevem perto dele precisa de uma linha
//!   so para si; senao, false sharing. Padding e desperdicio de cache, entao
//!   so vale nesse caso: contadores e indices de filas, estado por thread.
//!
//! Os valores sao fixados na compilacao por arquitetura: layout nao muda em
//! runtime, entao detectar o hardware na hora nao ajudaria.

use core::mem;
use core::ops::{Deref, DerefMut};

/// Tamanho da linha de cache: o que cabe junto aqui chega junto numa leitura.
#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
pub const CACHE_LINE: usize = 128;
/// Tamanho da linha de cache: o que cabe junto aqui chega junto numa leitura.
#[cfg(not(all(target_arch = "aarch64", target_vendor = "apple")))]
pub const CACHE_LINE: usize = 64;

/// Distancia minima entre dois dados escritos por threads diferentes para nao
/// dividirem linha. E o alinhamento de [`CacheAligned`].
///
/// Pode ser maior que [`CACHE_LINE`]: em x86_64 a linha tem 64 bytes, mas o
/// prefetcher espacial busca linhas em pares de 128, e duas threads em linhas
/// vizinhas do mesmo par ainda disputam.
pub const CACHE_ISOLATION: usize = mem::align_of::<CacheAligned<()>>();

/// `T` sozinho na(s) sua(s) linha(s) de cache: alinhado e com tamanho
/// arredondado para [`CACHE_ISOLATION`], entao nada mais divide linha com ele.
///
/// So para dado com escrita concorrente; em todo o resto, densidade.
///
/// `Deref` para `T`: `contador.fetch_add(1, ..)` funciona direto.
#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), repr(C, align(128)))]
#[cfg_attr(not(any(target_arch = "x86_64", target_arch = "aarch64")), repr(C, align(64)))]
#[derive(Default)]
pub struct CacheAligned<T>(pub T);

const _: () = {
    assert!(CACHE_ISOLATION >= CACHE_LINE, "isolamento menor que a linha nao isola");
    assert!(mem::size_of::<CacheAligned<u8>>() == CACHE_ISOLATION, "CacheAligned nao ocupa a linha inteira");
};

impl<T> CacheAligned<T> {
    #[inline]
    pub const fn new(value: T) -> CacheAligned<T> {
        CacheAligned(value)
    }

    #[inline]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for CacheAligned<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for CacheAligned<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
