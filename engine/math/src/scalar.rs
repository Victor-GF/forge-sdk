//! Funcoes de `f32` que o `core` nao tem.
//!
//! `f32::sqrt` e companhia vivem no `std`, que as delega para a libm do
//! sistema. Aqui, o que a CPU faz numa instrucao tem uma definicao por
//! arquitetura, com a mesma assinatura, e o `cfg` escolhe qual compila: quem
//! chama ve uma funcao so, igual em todo alvo, e o binario tem so a
//! instrucao daquele alvo.
//!
//! Sem versao em software escondida: uma arquitetura sem definicao aqui nao
//! compila. O resto e Rust puro, igual em toda arquitetura.

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("forge_math: sem implementacao de sqrt para esta arquitetura; suportadas: x86_64, aarch64.");

/// Raiz quadrada, corretamente arredondada (IEEE 754), em uma instrucao.
/// Negativo da NaN, como no `std`.
///
/// x86_64: `sqrtss`, do SSE, presente em todo processador de 64 bits.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
#[allow(unused_unsafe)]
pub fn sqrt(x: f32) -> f32 {
    use core::arch::x86_64::{_mm_cvtss_f32, _mm_set_ss, _mm_sqrt_ss};
    // Conforme a versao do compilador, as intrinsics de SSE sao `unsafe` ou
    // nao; SSE sempre existe neste alvo.
    unsafe { _mm_cvtss_f32(_mm_sqrt_ss(_mm_set_ss(x))) }
}

/// Raiz quadrada, corretamente arredondada (IEEE 754), em uma instrucao.
/// Negativo da NaN, como no `std`.
///
/// aarch64: `fsqrt`, em `asm!`, porque a intrinsic escalar nao existe em
/// Rust estavel.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn sqrt(x: f32) -> f32 {
    let result: f32;
    unsafe {
        core::arch::asm!(
            "fsqrt {result:s}, {x:s}",
            x = in(vreg) x,
            result = lateout(vreg) result,
            options(pure, nomem, nostack),
        );
    }
    result
}

/// Interpolacao linear: `a` com `t = 0`, `b` com `t = 1`. `t` fora de 0..1
/// extrapola.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqrt_exata_em_quadrados() {
        assert_eq!(sqrt(0.0), 0.0);
        assert_eq!(sqrt(1.0), 1.0);
        assert_eq!(sqrt(4.0), 2.0);
        assert_eq!(sqrt(1e6), 1e3);
    }

    #[test]
    fn sqrt_arredondada_como_ieee() {
        // O resultado correto de sqrt(2) em f32, bit a bit.
        assert_eq!(sqrt(2.0).to_bits(), core::f32::consts::SQRT_2.to_bits());
    }

    #[test]
    fn sqrt_de_negativo_e_nan() {
        assert!(sqrt(-1.0).is_nan());
    }

    #[test]
    fn lerp_nas_pontas_e_no_meio() {
        assert_eq!(lerp(2.0, 6.0, 0.0), 2.0);
        assert_eq!(lerp(2.0, 6.0, 1.0), 6.0);
        assert_eq!(lerp(2.0, 6.0, 0.5), 4.0);
    }
}
