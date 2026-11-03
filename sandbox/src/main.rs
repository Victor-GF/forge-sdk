//! Sandbox: aplicacao de teste da engine.
//!
//! Consome a engine como um jogo consumiria, para exercitar as
//! funcionalidades em runtime. Fica fora de `engine/`: nada da engine pode
//! depender daqui.
//!
//! Testa memoria uma vez na inicializacao, sobe um pool de threads, abre uma
//! janela e roda o loop ate o usuario fechar. A cada frame o pool recebe um
//! job por worker.

mod memory_check;
mod workers;

use forge_os::prelude::*;
use workers::Pool;

const FRAME_MS: u32 = 16;

fn main() {
    println!("[sandbox] thread principal {}", Platform::thread_current_id());

    memory_check::run();

    let pool = Pool::start(Platform::logical_processor_count().saturating_sub(1).max(1));

    let Some(window) = Platform::window_create(c"forge sandbox", 1280, 720) else {
        Platform::message_box(c"forge sandbox", c"Falha ao criar a janela.");
        Platform::exit_process(1);
    };

    let mut frames: u64 = 0;
    let mut size = (0, 0);
    while !Platform::window_close_requested(&window) {
        Platform::window_pump_events();

        let current = Platform::window_size(&window);
        if current != size {
            size = current;
            println!("[window] {}x{}", size.0, size.1);
        }

        pool.run(pool.len());

        frames += 1;
        Platform::thread_sleep(FRAME_MS);
    }

    Platform::window_destroy(window);
    let jobs = pool.stop();
    println!("[sandbox] {frames} frames, {jobs} jobs");
}
