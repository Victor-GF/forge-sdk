//! Custo fixo do pool por job: lotes de jobs que nao fazem nada.
//!
//! Sem trabalho util, o que sobra e o mecanismo: pegar o indice (CAS em
//! `next`), ler o lote, chamar a funcao e terminar (`fetch_sub` em
//! `remaining`). E onde layout de cache e contencao aparecem: para comparar
//! um ajuste no pool, rode antes e depois e compare o ns/job.

use crate::workers::{Job, Pool};
use core::ffi::c_void;
use core::ptr;
use forge_os::prelude::*;

const JOBS: usize = 1 << 16;
const ROUNDS: u32 = 32;

fn empty(_: *mut c_void) {}

pub fn run(pool: &Pool) {
    let jobs: Vec<Job> = (0..JOBS).map(|_| Job { func: empty, data: ptr::null_mut() }).collect();

    // Aquece: threads acordadas uma vez, `jobs` ja em cache.
    pool.run(&jobs);

    let start = Platform::time_ticks();
    for _ in 0..ROUNDS {
        pool.run(&jobs);
    }
    let nanos = Platform::time_ticks_to_nanos(Platform::time_ticks() - start);

    let total = JOBS as u64 * ROUNDS as u64;
    log!(
        "[bench] {ROUNDS} lotes de {JOBS} jobs vazios: {:.1} ns/job, {:.1} us/lote",
        nanos as f64 / total as f64,
        nanos as f64 / ROUNDS as f64 / 1000.0,
    );
}
