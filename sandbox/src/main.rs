//! Sandbox: aplicacao de teste da engine.
//!
//! Consome a engine como um jogo consumiria, para exercitar as
//! funcionalidades em runtime. Fica fora de `engine/`: nada da engine pode
//! depender daqui.
//!
//! Loga a CPU e testa memoria, arquivos e sincronizacao na inicializacao,
//! sobe um pool de threads, abre uma janela e roda o loop ate o usuario
//! fechar. A cada frame o pool soma um array em pedacos, divididos entre os
//! workers e a thread principal, e confere o resultado. O ritmo e de
//! `FRAME_RATE` frames por segundo, com espera precisa. O codigo do jogo
//! (`sandbox/game`) roda numa DLL recarregada quando recompilada
//! (`hot_reload`).

mod audio_demo;
mod file_check;
mod hot_reload;
mod memory_check;
mod parallel_sum;
mod pool_bench;
mod sync_check;
mod workers;

use audio_demo::AudioDemo;
use forge_core::cache::CACHE_LINE;
use forge_os::prelude::*;
use hot_reload::GameCode;
use parallel_sum::ParallelSum;
use workers::Pool;

/// Frames por segundo do loop, com espera precisa (`timer_wait_until`).
const FRAME_RATE: u64 = 60;
const SUM_VALUES: usize = 1 << 20;
const JOBS_PER_WORKER: usize = 4;
/// Memoria do jogo: do executavel, sobrevive ao hot reload.
const GAME_MEMORY: usize = 1 << 20;
/// Nome que o cargo da ao cdylib no Windows: sem prefixo `lib`.
const GAME_DLL: &str = "forge_sandbox_game.dll";

fn main() {
    // Panic vira abort (`panic = "abort"`); antes disso, a mensagem vai para o
    // console e para o debugger, e com debugger anexado o processo para na
    // linha do panic em vez de sumir.
    std::panic::set_hook(Box::new(|info| {
        forge_log!("[panic] {info}");
        if Platform::debugger_present() {
            Platform::debug_break();
        }
    }));

    Platform::thread_set_name("forge main");
    forge_log!("[sandbox] thread principal {}", Platform::thread_current_id());
    forge_log!("[time] contador a {} ticks/s", Platform::time_frequency());
    forge_log!("[sandbox] acentuação: çãé ü → ok");

    // Testa o caminho de panic: log, `debug_break` com debugger, abort.
    if std::env::args().any(|arg| arg == "--panic") {
        panic!("panic de teste pedido por --panic (çã)");
    }

    log_cpu();
    memory_check::run();
    file_check::run();
    sync_check::run();

    // A thread principal tambem executa jobs em `run`: um worker a menos.
    let pool = Pool::start(Platform::logical_processor_count().saturating_sub(1).max(1));
    pool_bench::run(&pool);
    let mut sum = ParallelSum::new(SUM_VALUES, pool.len() as usize * JOBS_PER_WORKER);

    let game_memory = Platform::pages_alloc(GAME_MEMORY);
    assert!(!game_memory.is_null(), "pages_alloc da memoria do jogo falhou");
    let mut game = GameCode::new(GAME_DLL);

    let Some(window) = Platform::window_create(c"forge sandbox", 1280, 720) else {
        Platform::message_box(c"forge sandbox", c"Falha ao criar a janela.");
        Platform::exit_process(1);
    };
    forge_log!(
        "[window] escala {:.2}, {:?}",
        Platform::window_dpi_scale(&window),
        Platform::window_native(&window)
    );
    forge_log!("[window] F11 ou Alt+Enter alterna tela cheia");

    let timer = Platform::timer_create().expect("timer_create falhou");
    let frame_ticks = Platform::time_frequency() / FRAME_RATE;
    let mut next_frame = Platform::time_ticks() + frame_ticks;
    let mut last_frame = Platform::time_ticks();
    let mut frame_time = Timing::new();
    let mut audio = AudioDemo::new(FRAME_RATE);

    let mut frames: u64 = 0;
    let mut size = (0, 0);
    // Os gamepads comparam com o frame anterior: o mesmo `Input` vive o
    // loop inteiro.
    let mut input = Input::default();
    let mut sum_time = Timing::new();
    while !Platform::window_close_requested(&window) {
        Platform::window_pump_events();

        let current = Platform::window_size(&window);
        if current != size {
            size = current;
            forge_log!("[window] {}x{}", size.0, size.1);
        }

        Platform::window_input(&window, &mut input.keyboard, &mut input.mouse);
        Platform::gamepads_update(&mut input.gamepads);

        let keyboard = &input.keyboard;
        let alt = keyboard.key(Key::LeftAlt).down || keyboard.key(Key::RightAlt).down;
        if keyboard.key(Key::F11).pressed() || (alt && keyboard.key(Key::Enter).pressed()) {
            let fullscreen = !Platform::window_is_fullscreen(&window);
            Platform::window_set_fullscreen(&window, fullscreen);
            forge_log!("[window] tela cheia: {fullscreen}");
        }
        audio.update(keyboard);

        game.poll();
        // `pages_alloc` entrega memoria zerada e alinhada a pagina, so do jogo.
        unsafe { game.update(game_memory, GAME_MEMORY, &input, frames) };

        let start = Platform::time_ticks();
        sum.run(&pool);
        sum_time.add(Platform::time_ticks() - start);

        frames += 1;

        // Prazo absoluto: um frame que acordou atrasado nao empurra os
        // seguintes. Mais de um frame de atraso (janela sendo arrastada,
        // breakpoint) recomeca a contagem a partir de agora.
        Platform::timer_wait_until(&timer, next_frame);
        let now = Platform::time_ticks();
        frame_time.add(now - last_frame);
        last_frame = now;
        next_frame += frame_ticks;
        if now >= next_frame {
            next_frame = now + frame_ticks;
        }
    }

    drop(audio);
    Platform::window_destroy(window);
    drop(game);
    unsafe { Platform::pages_release(game_memory) };
    let jobs = pool.stop();
    forge_log!("[sandbox] {frames} frames, {jobs} jobs");
    sum_time.print("sum.run");
    frame_time.print("frame");
    Platform::timer_destroy(timer);
}

/// Topologia da CPU, e se o `CACHE_LINE` de compilacao bate com a linha real.
fn log_cpu() {
    let Some(cpu) = Platform::cpu_topology() else {
        forge_log!("[cpu] topologia indisponivel");
        return;
    };
    forge_log!(
        "[cpu] {} nucleos fisicos ({} de desempenho), {} logicos",
        cpu.physical_cores,
        cpu.performance_cores,
        cpu.logical_processors
    );
    forge_log!(
        "[cpu] L1d {} KB, L2 {} KB, L3 {} KB x{}, linha {} B",
        cpu.l1_data / 1024,
        cpu.l2 / 1024,
        cpu.l3 / 1024,
        cpu.l3_count,
        cpu.cache_line
    );
    if cpu.cache_line != 0 && cpu.cache_line as usize != CACHE_LINE {
        forge_log!("[cpu] aviso: CACHE_LINE e {CACHE_LINE}, mas a linha real e {}", cpu.cache_line);
    }
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
        forge_log!(
            "[time] {name}: media {:.1} us, min {:.1} us, max {:.1} us ({} medidas)",
            us(self.total / self.count),
            us(self.min),
            us(self.max),
            self.count,
        );
    }
}
