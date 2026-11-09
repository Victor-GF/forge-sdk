//! Matematica da engine: por enquanto [`Vec3`], [`Vec4`] e as funcoes
//! escalares que eles precisam.
//!
//! `no_std` e sem contato com o SO: o mesmo codigo roda no jogo, nas
//! ferramentas e em qualquer plataforma. Depende so do `forge_core`.
//!
//! As poucas operacoes que sao uma instrucao da CPU (raiz quadrada) tem uma
//! definicao por arquitetura, escolhida pelo `cfg` em tempo de
//! compilacao (ver [`scalar`]). A API publica nao tem `cfg`: quem usa nunca
//! ve arquitetura.
//!
//! # Convencoes
//!
//! - Sistema de coordenadas destro, com Y para cima, X para a direita e -Z
//!   para a frente. E o do glTF, o formato que o Blender exporta: modelos
//!   chegam sem conversao de eixos.
//! - Tipos `#[repr(C)]` com layout travado: podem ir como estao para a GPU,
//!   para um arquivo ou para o codigo do jogo numa DLL recarregada.
//! - Operacoes invalidas (normalizar um vetor nulo) sao bug de quem chama:
//!   `debug_assert!` em debug, resultado sem sentido (NaN) em release. Onde o
//!   caso e esperado, ha uma versao que nao falha (`normalize_or_zero`).

#![no_std]

mod ops;
pub mod scalar;
mod vec3;
mod vec4;

pub use vec3::Vec3;
pub use vec4::Vec4;
