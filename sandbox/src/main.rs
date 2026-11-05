//! Sandbox: aplicacao de teste da engine.
//!
//! Consome a engine como um jogo consumiria, para exercitar as
//! funcionalidades em runtime. Fica fora de `engine/`: nada da engine pode
//! depender daqui.
//!
//! Testa memoria uma vez na inicializacao, sobe um pool de threads, abre uma
//! janela e roda o loop ate o usuario fechar. A cada frame o pool soma um
//! array em pedacos, divididos entre os workers e a thread principal, e
//! confere o resultado.

mod memory_check;
mod parallel_sum;
mod pool_bench;
mod workers;

use forge_os::prelude::*;
use parallel_sum::ParallelSum;
use workers::Pool;

const FRAME_MS: u32 = 16;
const SUM_VALUES: usize = 1 << 20;
const JOBS_PER_WORKER: usize = 4;

fn main() {
    // Panic vira abort (`panic = "abort"`); antes disso, a mensagem vai para o
    // console e para o debugger, e com debugger anexado o processo para na
    // linha do panic em vez de sumir.
    std::panic::set_hook(Box::new(|info| {
        log!("[panic] {info}");
        if Platform::debugger_present() {
            Platform::debug_break();
        }
    }));

    Platform::thread_set_name("forge main");
    log!("[sandbox] thread principal {}", Platform::thread_current_id());
    log!("[time] contador a {} ticks/s", Platform::time_frequency());

    memory_check::run();

    // A thread principal tambem executa jobs em `run`: um worker a menos.
    let pool = Pool::start(Platform::logical_processor_count().saturating_sub(1).max(1));
    pool_bench::run(&pool);
    let mut sum = ParallelSum::new(SUM_VALUES, pool.len() as usize * JOBS_PER_WORKER);

    let Some(window) = Platform::window_create(c"forge sandbox", 1280, 720) else {
        Platform::message_box(c"forge sandbox", c"Falha ao criar a janela.");
        Platform::exit_process(1);
    };

    let mut frames: u64 = 0;
    let mut size = (0, 0);
    let mut sum_time = Timing::new();
    while !Platform::window_close_requested(&window) {
        Platform::window_pump_events();

        let current = Platform::window_size(&window);
        if current != size {
            size = current;
            log!("[window] {}x{}", size.0, size.1);
        }

        let start = Platform::time_ticks();
        sum.run(&pool);
        sum_time.add(Platform::time_ticks() - start);

        frames += 1;
        Platform::thread_sleep(FRAME_MS);
    }

    Platform::window_destroy(window);
    let jobs = pool.stop();
    log!("[sandbox] {frames} frames, {jobs} jobs");
    sum_time.print("sum.run");
}

/// Minimo, media e maximo de uma medida repetida, em ticks.
struct Timing {
    count: u64,
    total: u64,
    min: u64,
    max: u64,
}

impl Timing {
    fn new() -> Timing {
        Timing { count: 0, total: 0, min: u64::MAX, max: 0 }
    }

    fn add(&mut self, ticks: u64) {
        self.count += 1;
        self.total += ticks;
        self.min = self.min.min(ticks);
        self.max = self.max.max(ticks);
    }

    fn print(&self, name: &str) {
        if self.count == 0 {
            return;
        }
        let us = |ticks: u64| Platform::time_ticks_to_nanos(ticks) as f64 / 1000.0;
        log!(
            "[time] {name}: media {:.1} us, min {:.1} us, max {:.1} us ({} medidas)",
            us(self.total / self.count),
            us(self.min),
            us(self.max),
            self.count,
        );
    }
}
