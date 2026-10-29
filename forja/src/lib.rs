//! forja: um núcleo de engine em branco.
//!
//! A engine não traz física nem renderer. Ela traz o "banco de dados" do projeto:
//! tabelas declaradas (layout, tempo de vida, invariantes) e sistemas que declaram
//! o que leem e escrevem. A partir dessas declarações ela monta o grafo do frame,
//! executa em paralelo com segurança, mede, analisa e sabe se recuperar de falhas.

pub mod analysis;
pub mod crash;
pub mod draw;
pub mod editor;
pub mod jobs;
pub mod math;
pub mod platform;
pub mod png;
pub mod profiler;
pub mod project;
pub mod schedule;
pub mod world;
