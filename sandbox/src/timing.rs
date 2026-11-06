//! Estatistica de uma medida repetida (tempo de frame, de `sum.run`).
//!
//! Guarda todas as amostras: media e maximo escondem o que importa num jogo,
//! que e a cauda. Um frame em cem que passa do orcamento e um engasgo
//! visivel, mesmo com a media perfeita. Por isso o resumo traz percentis e
//! quantas amostras passaram do orcamento.

use forge_os::prelude::*;

pub struct Timing {
    /// Em ticks de `time_ticks`.
    samples: Vec<u64>,
}

impl Timing {
    pub fn new() -> Timing {
        Timing { samples: Vec::new() }
    }

    #[inline]
    pub fn add(&mut self, ticks: u64) {
        self.samples.push(ticks);
    }

    /// Loga media, minimo, percentis e maximo, em microssegundos. Com
    /// `budget`, tambem quantas amostras passaram dele.
    pub fn print(&self, name: &str, budget: Option<u64>) {
        if self.samples.is_empty() {
            return;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let us = |ticks: u64| Platform::time_ticks_to_nanos(ticks) as f64 / 1000.0;
        let percentile = |p: usize| sorted[(sorted.len() - 1) * p / 100];
        let mean = sorted.iter().sum::<u64>() / sorted.len() as u64;

        forge_log!(
            "[time] {name}: media {:.1} us | min {:.1} | p50 {:.1} | p95 {:.1} | p99 {:.1} | max {:.1} ({} medidas)",
            us(mean),
            us(sorted[0]),
            us(percentile(50)),
            us(percentile(95)),
            us(percentile(99)),
            us(sorted[sorted.len() - 1]),
            sorted.len(),
        );
        if let Some(budget) = budget {
            let over = sorted.iter().filter(|&&ticks| ticks > budget).count();
            forge_log!("[time] {name}: {over} acima de {:.1} us", us(budget));
        }
    }
}
