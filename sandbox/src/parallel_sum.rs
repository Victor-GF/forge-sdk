//! Job de exemplo para o pool: soma um array grande em pedacos, um job por
//! pedaco, e confere com a soma feita uma vez na thread principal.
//!
//! Cada pedaco le so a sua faixa do array e escreve so no proprio `sum`:
//! jobs do mesmo lote nunca escrevem na mesma memoria.

use crate::workers::{Job, Pool};
use core::ffi::c_void;
use core::slice;
use forge_os::forge_log;

/// Dados de um job: a faixa a somar e onde guardar o resultado.
struct Chunk {
    values: *const u32,
    len: usize,
    sum: u64,
}

fn sum_chunk(data: *mut c_void) {
    let chunk = unsafe { &mut *(data as *mut Chunk) };
    let values = unsafe { slice::from_raw_parts(chunk.values, chunk.len) };
    chunk.sum = values.iter().map(|&v| v as u64).sum();
}

pub struct ParallelSum {
    values: Vec<u32>,
    expected: u64,
    chunk_count: usize,
    /// `chunks` e `jobs` sao refeitos a cada `run`, mas mantem a capacidade:
    /// so alocam no primeiro.
    chunks: Vec<Chunk>,
    jobs: Vec<Job>,
}

impl ParallelSum {
    pub fn new(len: usize, chunk_count: usize) -> ParallelSum {
        assert!(chunk_count > 0, "precisa de pelo menos um pedaco");
        let values: Vec<u32> = (0..len as u32).collect();
        let expected = values.iter().map(|&v| v as u64).sum();
        forge_log!("[sum] {len} valores em {chunk_count} jobs por frame");
        ParallelSum {
            values,
            expected,
            chunk_count,
            chunks: Vec::with_capacity(chunk_count),
            jobs: Vec::with_capacity(chunk_count),
        }
    }

    /// Soma o array no pool e confere o resultado.
    pub fn run(&mut self, pool: &Pool) {
        let len = self.values.len();
        self.chunks.clear();
        for i in 0..self.chunk_count {
            let start = i * len / self.chunk_count;
            let end = (i + 1) * len / self.chunk_count;
            let range = &self.values[start..end];
            self.chunks.push(Chunk { values: range.as_ptr(), len: range.len(), sum: 0 });
        }

        // `chunks` nao muda de tamanho daqui ate o fim de `pool.run`: os
        // ponteiros dos jobs continuam validos.
        self.jobs.clear();
        for chunk in self.chunks.iter_mut() {
            self.jobs.push(Job { func: sum_chunk, data: (chunk as *mut Chunk).cast() });
        }

        pool.run(&self.jobs);

        let total: u64 = self.chunks.iter().map(|c| c.sum).sum();
        assert_eq!(total, self.expected, "soma paralela diverge da sequencial");
    }
}
