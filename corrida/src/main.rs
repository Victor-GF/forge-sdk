//! corrida: fatia vertical sobre a forja.
//!
//!   cargo run --release -p corrida                      jogar
//!   cargo run --release -p corrida -- --autopilot       o carro dirige sozinho
//!   cargo run --release -p corrida -- --replay <log>    reproduz um input.log de relatório
//!   cargo run --release -p corrida -- --analyze [ticks] roda headless e gera reports/analysis.md

mod hud;
mod render;
mod systems;
mod tables;
mod track;

use std::path::{Path, PathBuf};
use std::time::Instant;

use forja::analysis;
use forja::crash;
use forja::draw::Canvas;
use forja::editor::{Editor, View};
use forja::jobs::JobPool;
use forja::platform::{vk, Text, Window};
use forja::profiler;
use forja::project::{self, Level, Project};
use forja::schedule::{RunOpts, Schedule, SysFn, TickInfo};
use forja::world::World;

use render::DT;
use tables::*;
use track::{StaticScene, Track};

const W: usize = 1280;
const H: usize = 720;

struct Args {
    replay: Option<PathBuf>,
    analyze: Option<u64>,
    autopilot: bool,
    workers: Option<usize>,
    projeto: Option<PathBuf>,
    shots: Vec<u64>,
    shot_view: View,
    force_crash: Option<u64>,
    inject: Vec<(u64, u32)>,
}

fn parse_args() -> Args {
    let mut a = Args { replay: None, analyze: None, autopilot: false, workers: None, projeto: None, shots: Vec::new(), shot_view: View::Off, force_crash: None, inject: Vec::new() };
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--replay" => a.replay = it.next().map(PathBuf::from),
            "--autopilot" => a.autopilot = true,
            "--workers" => a.workers = it.next().and_then(|s| s.parse().ok()),
            "--projeto" => a.projeto = it.next().map(PathBuf::from),
            "--shot" => a.shots.extend(it.next().and_then(|s| s.parse::<u64>().ok())),
            "--inject" => {
                // <tick>:<nan|vfx|rules|av|hang>  -> mesma coisa que apertar F5..F9 naquele tick
                let spec = it.next().unwrap_or_default();
                let (t, f) = spec.split_once(':').unwrap_or(("", ""));
                let bit = match f {
                    "nan" => F_NAN,
                    "vfx" => F_PANIC_VFX,
                    "rules" => F_PANIC_RULES,
                    "av" => F_AV,
                    "hang" => F_HANG,
                    _ => 0,
                };
                match t.parse::<u64>() {
                    Ok(t) if bit != 0 => a.inject.push((t, bit)),
                    _ => {
                        eprintln!("--inject espera <tick>:<nan|vfx|rules|av|hang>, recebeu '{}'", spec);
                        std::process::exit(2);
                    }
                }
            }
            "--force-crash" => a.force_crash = it.next().and_then(|s| s.parse().ok()),
            "--shot-view" => {
                a.shot_view = match it.next().as_deref() {
                    Some("timeline") => View::Timeline,
                    Some("matriz") => View::Matrix,
                    Some("tabelas") => View::Tables,
                    Some("falhas") => View::Failures,
                    _ => View::Off,
                }
            }
            "--analyze" => {
                let n = it.peek().and_then(|s| s.parse::<u64>().ok());
                if n.is_some() {
                    it.next();
                }
                a.analyze = Some(n.unwrap_or(3600));
            }
            other => {
                eprintln!("argumento desconhecido: {}", other);
                std::process::exit(2);
            }
        }
    }
    a
}

/// Onde gravar o perfil e quais funções o código oferece.
struct Output {
    perfil: PathBuf,
    implemented: Vec<&'static str>,
}

impl Output {
    fn save(&self, world: &World, sim: &Schedule, render: &Schedule, origin: &str) {
        match project::write_profile(&self.perfil, world, &[sim, render], &self.implemented, origin) {
            Ok(()) => eprintln!("[corrida] perfil gravado em {}", self.perfil.display()),
            Err(e) => eprintln!("[corrida] falha ao gravar {}: {}", self.perfil.display(), e),
        }
    }
}

/// `--projeto`, ou corrida/projeto.forja a partir do diretório atual, ou relativo ao executável.
fn find_project(arg: Option<PathBuf>) -> PathBuf {
    if let Some(p) = arg {
        return p;
    }
    let mut cands = vec![PathBuf::from("corrida/projeto.forja"), PathBuf::from("projeto.forja")];
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        cands.push(dir.join("../../corrida/projeto.forja"));
    }
    cands.iter().find(|p| p.exists()).cloned().unwrap_or_else(|| cands[0].clone())
}

/// Lê o pipeline do arquivo, valida contra as tabelas e funções do código e monta os grafos.
/// Recusa rodar com erros: os mesmos que o forja-editor mostra em vermelho.
fn load_pipeline(path: &Path, world: &World, fns: &[(&'static str, SysFn)]) -> (Schedule, Schedule) {
    let fail = |title: String, lines: Vec<String>| -> ! {
        eprintln!("{}", title);
        for l in &lines {
            eprintln!("  - {}", l);
        }
        if std::env::args().all(|a| a != "--analyze") {
            forja::platform::message_box("corrida: pipeline inválido", &format!("{}

{}", title, lines.join("
")));
        }
        std::process::exit(2);
    };
    let p = Project::load(path).unwrap_or_else(|e| fail(format!("Não consegui ler o pipeline em {}:", path.display()), e));
    let implemented: Vec<String> = fns.iter().map(|f| f.0.to_string()).collect();
    let issues = project::validate(&p, &analysis::table_models(world), &implemented);
    let errors: Vec<String> = issues.iter().filter(|i| i.level == Level::Error).map(|i| i.msg.clone()).collect();
    if !errors.is_empty() {
        fail(format!("{} tem {} erro(s):", path.display(), errors.len()), errors);
    }
    for i in issues.iter().filter(|i| i.level == Level::Warning) {
        eprintln!("[corrida] aviso: {}", i.msg);
    }
    let sim_spec = p.graphs.iter().find(|g| !g.render).unwrap_or_else(|| fail("O projeto não tem grafo de simulação (render = nao)".into(), vec![]));
    let render_spec = p.graphs.iter().find(|g| g.render).unwrap_or_else(|| fail("O projeto não tem grafo de render (render = sim)".into(), vec![]));
    let sim = project::build_schedule(sim_spec, world, fns, 0).unwrap_or_else(|e| fail(format!("Grafo '{}' inválido:", sim_spec.name), e));
    let render = project::build_schedule(render_spec, world, fns, sim.systems.len() as u16).unwrap_or_else(|e| fail(format!("Grafo '{}' inválido:", render_spec.name), e));
    eprintln!("[corrida] pipeline: {} ({} + {} sistemas)", path.display(), sim.systems.len(), render.systems.len());
    (sim, render)
}

/// Contagem de eventos por segundo e quem os produziu (visão produtor -> consumidor).
struct EventStats {
    window: [u32; EVENT_KINDS.len()],
    shown: [u32; EVENT_KINDS.len()],
    producers: Vec<Vec<u16>>,
    since: Instant,
}

impl EventStats {
    fn new() -> EventStats {
        EventStats { window: [0; EVENT_KINDS.len()], shown: [0; EVENT_KINDS.len()], producers: vec![Vec::new(); EVENT_KINDS.len()], since: Instant::now() }
    }

    fn observe(&mut self, world: &mut World) {
        for e in &world.get::<Events>().e {
            let k = e.kind as usize;
            self.window[k] += 1;
            if !self.producers[k].contains(&e.src) {
                self.producers[k].push(e.src);
            }
        }
        if self.since.elapsed().as_secs_f32() >= 1.0 {
            self.shown = self.window;
            self.window = [0; EVENT_KINDS.len()];
            self.since = Instant::now();
        }
    }

    fn lines(&self, world: &World, sim: &Schedule) -> Vec<String> {
        let name = |id: u16| sim.systems.get((id - sim.base_id) as usize).map(|s| s.decl.name).unwrap_or("?");
        let mut out = vec![format!(
            "Eventos/s: {}",
            EVENT_KINDS.iter().enumerate().map(|(i, k)| format!("{:?} {}", k, self.shown[i])).collect::<Vec<_>>().join("  ")
        )];
        for (i, k) in EVENT_KINDS.iter().enumerate() {
            if !self.producers[i].is_empty() {
                out.push(format!("  {:?} <- {}", k, self.producers[i].iter().map(|&s| name(s)).collect::<Vec<_>>().join(", ")));
            }
        }
        let ev = world.id_of::<Events>();
        let consumers: Vec<&str> = sim.systems.iter().filter(|s| s.reads.contains(&ev) && !s.writes.contains(&ev)).map(|s| s.decl.name).collect();
        out.push(format!("  consumidores declarados de Events: {}", consumers.join(", ")));
        let (alive, dropped) = world.with::<Particles, _>(|p| (p.pos.len(), p.dropped));
        out.push(format!("Partículas vivas: {} / {}   descartadas por pool cheio: {}", alive, PARTICLE_CAP, dropped));
        let crashes: Vec<String> = world.with::<CrashState, _>(|c| c.c.iter().enumerate().map(|(i, x)| format!("{} {}", CAR_NAMES[i], x.count)).collect());
        out.push(format!("Crashes: {}", crashes.join("  ")));
        out
    }
}

fn main() {
    let args = parse_args();
    crash::install("reports", "corrida", args.analyze.is_none());

    let workers = args.workers.unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(1).clamp(1, 7));
    let pool = JobPool::new(workers);

    let t0 = Instant::now();
    let track = Track::cook();
    let clearance = track.min_clearance();
    let samples = track.n();
    let length = track.length;
    let scene = StaticScene::bake(&track, &pool);
    let cook_ms = t0.elapsed().as_secs_f64() * 1e3;
    let cook = format!(
        "cook: pista {:.0} m ({} amostras, menor distância entre trechos {:.0} m), cenário {}x{} em {:.0} ms com {} lanes",
        length,
        samples,
        clearance,
        scene.w,
        scene.h,
        cook_ms,
        workers + 1
    );
    eprintln!("[corrida] {}", cook);
    crash::breadcrumb(cook.clone());
    if clearance < 2.0 * track::WALL + 4.0 {
        eprintln!("[corrida] aviso: trechos da pista a {:.0} m um do outro; os muros podem se sobrepor", clearance);
    }

    let mut world = World::new();
    tables::register(&mut world, track, scene, W, H);
    let mut fns: Vec<(&'static str, SysFn)> = Vec::new();
    systems::register(&mut fns);
    render::register(&mut fns);
    let projeto = find_project(args.projeto.clone());
    let (sim, render) = load_pipeline(&projeto, &world, &fns);
    let out = Output { perfil: projeto.with_file_name("perfil.forja"), implemented: fns.iter().map(|f| f.0).collect() };

    if let Some(path) = &args.replay {
        match crash::load_input_log(path) {
            Ok(log) => {
                eprintln!("[corrida] replay: {} ticks, {} checkpoints de hash", log.inputs.len(), log.hashes.len());
                crash::breadcrumb(format!("replay de {}", path.display()));
                let src = world.get_mut::<InputSource>();
                src.replay_len = log.inputs.len();
                src.replay = Some(log);
            }
            Err(e) => {
                eprintln!("não consegui ler o log de replay {}: {}", path.display(), e);
                std::process::exit(2);
            }
        }
    }
    // o estado inicial do piloto automático entra no log como um input do tick 0
    if (args.autopilot || args.analyze.is_some()) && args.replay.is_none() {
        world.get_mut::<InputSource>().pressed_bits |= IN_AUTO;
    }

    match args.analyze {
        Some(ticks) => analyze(&mut world, &sim, &render, &pool, ticks, &cook, &args, &out),
        None => run_window(&mut world, &sim, &render, &pool, &out),
    }
}

fn run_window(world: &mut World, sim: &Schedule, render: &Schedule, pool: &JobPool, out: &Output) {
    let prof = profiler::get();
    let mut win = Window::open("forja · corrida (MVP)", W, H);
    let mut editor = Editor::new();
    let mut stats = EventStats::new();
    let mut texts: Vec<Text> = Vec::new();
    let mut spare = vec![0u32; W * H];
    let mut tick: u64 = 0;
    let mut acc = 0.0f32;
    let mut ts = 1.0f32;
    let mut last = Instant::now();
    let mut title_at = Instant::now();
    let mut frames = 0u64;
    let mut perfil_at = Instant::now();

    loop {
        prof.begin_frame();
        win.pump();
        if win.quit || win.keys.pressed(vk::ESC) {
            break;
        }
        let n_sys = sim.systems.len() + render.systems.len();
        if editor.handle_keys(&win.keys, n_sys) {
            let n = sim.enable_all() + render.enable_all();
            crash::breadcrumb(format!("{} sistema(s) religado(s)", n));
        }

        // input bruto do frame -> InputSource (o sistema Input.Sample transforma em dados do tick)
        {
            let k = &win.keys;
            let mut live = 0;
            for (key, bit) in [(vk::UP, IN_UP), (vk::DOWN, IN_DOWN), (vk::LEFT, IN_LEFT), (vk::RIGHT, IN_RIGHT), (vk::SPACE, IN_HAND), (vk::SHIFT, IN_BOOST)] {
                if k.down(key) {
                    live |= bit;
                }
            }
            let mut pressed = 0;
            for (key, bit) in [(b'T', IN_AUTO), (vk::F5, F_NAN), (vk::F6, F_PANIC_VFX), (vk::F7, F_PANIC_RULES), (vk::F8, F_AV), (vk::F9, F_HANG)] {
                if k.pressed(key) {
                    pressed |= bit;
                }
            }
            if pressed & FAULTS != 0 {
                crash::breadcrumb(format!("falha injetada pelo teclado: bits {:#x}", pressed & FAULTS));
            }
            let src = world.get_mut::<InputSource>();
            src.live_bits = live;
            src.pressed_bits |= pressed;
        }
        if win.keys.pressed(b'L') {
            let t = forja::platform::local_time();
            let path = PathBuf::from("reports").join(format!("replay_{:02}-{:02}-{:02}.log", t.wHour, t.wMinute, t.wSecond));
            match crash::save_input_log(&path) {
                Ok(()) => crash::breadcrumb(format!("replay salvo em {}", path.display())),
                Err(e) => crash::breadcrumb(format!("falha ao salvar replay: {}", e)),
            }
        }

        // relógios: real -> jogo (timescale pedido pelo Diretor) -> ticks fixos de simulação
        let now = Instant::now();
        let real_dt = (now - last).as_secs_f32().min(0.1);
        last = now;
        let target = world.get::<TimeControl>().target;
        ts += (target - ts) * (real_dt * 8.0).min(1.0);
        acc += real_dt * ts;
        let opts = RunOpts { invariants: editor.invariants };
        let mut ticks = 0u8;
        while acc >= DT && ticks < 4 {
            world.clear_transients();
            sim.run(world, pool, TickInfo { tick, dt: DT, sub: ticks, alpha: 0.0 }, opts);
            stats.observe(world);
            tick += 1;
            ticks += 1;
            acc -= DT;
        }
        if ticks == 4 {
            acc = acc.min(DT);
        }
        let alpha = (acc / DT).clamp(0.0, 1.0);
        render.run(world, pool, TickInfo { tick, dt: real_dt, sub: 0, alpha }, opts);

        // HUD e editor desenham por cima, fora do grafo
        texts.clear();
        std::mem::swap(&mut world.get_mut::<Framebuffer>().px, &mut spare);
        {
            let mut canvas = Canvas::full(W, H, &mut spare);
            let hud = world.with::<RenderSnapshot, _>(|s| s.hud);
            let replay = world.with::<InputSource, _>(|s| (s.replay.is_some(), s.replay_len, s.divergence, s.checked));
            let autopilot = world.with::<PlayerIntent, _>(|p| p.autopilot);
            if !full_panel(editor.view) {
                hud::draw(&mut canvas, &mut texts, world, &hud, replay, tick, editor.view == View::Off, autopilot);
            }
            editor.extra = stats.lines(world, sim);
            editor.extra.push(format!("tick {}  timescale {:.2}  ticks neste frame {}", tick, ts, ticks));
            editor.draw(&mut canvas, &mut texts, world, &[sim, render]);
        }
        win.present(&spare, &texts);
        std::mem::swap(&mut world.get_mut::<Framebuffer>().px, &mut spare);

        let idx = prof.end_frame(ticks, ts);
        crash::heartbeat(idx + 1);
        frames += 1;
        if frames > 120 {
            if let Some(f) = prof.last_frame() {
                let ms = f.dur() as f64 / 1e6;
                if ms > 50.0 {
                    let mut zs: Vec<_> = f.zones.iter().filter(|z| z.sub == profiler::NO_SUB).collect();
                    zs.sort_by_key(|z| std::cmp::Reverse(z.dur()));
                    let top: Vec<String> = zs.iter().take(6).map(|z| format!("  {:<20} {:7.2} ms (lane {})", z.name, z.dur() as f64 / 1e6, z.lane)).collect();
                    crash::spike(&format!("frame {} levou {:.1} ms (orçamento 16.7 ms), {} ticks.\nZonas mais caras:\n{}", f.index, ms, f.ticks, top.join("\n")));
                }
            }
        }
        if title_at.elapsed().as_secs_f32() > 0.5 {
            title_at = Instant::now();
            if let Some(f) = prof.last_frame() {
                win.set_title(&format!("forja · corrida (MVP)   {:.1} ms/frame   tick {}", f.dur() as f64 / 1e6, tick));
            }
        }
        if perfil_at.elapsed().as_secs() >= 30 {
            perfil_at = Instant::now();
            out.save(world, sim, render, &format!("corrida (janela), {} frames", frames));
        }
    }
    out.save(world, sim, render, &format!("corrida (janela), {} frames", frames));
}

#[allow(clippy::too_many_arguments)]
fn analyze(world: &mut World, sim: &Schedule, render: &Schedule, pool: &JobPool, ticks: u64, cook: &str, args: &Args, out: &Output) {
    let prof = profiler::get();
    println!("Rodando {} ticks headless ({:.0} s de jogo), todos os carros no piloto automático...", ticks, ticks as f32 * DT);
    let opts = RunOpts { invariants: true };
    let t = Instant::now();
    let mut renders = 0;
    let mut sim_walls: Vec<f64> = Vec::with_capacity(ticks as usize);
    let mut editor = Editor::new();
    editor.view = args.shot_view;
    for tick in 0..ticks {
        prof.begin_frame();
        world.clear_transients();
        if args.force_crash == Some(tick) {
            // evento sintético: exercita a cadeia Crash.Update -> Director -> VFX -> render
            let p = world.get::<VehicleHot>().pos[0];
            let v = world.get::<VehicleHot>().vel[0];
            world.get_mut::<Events>().e.push(Event { kind: EventKind::Crash, car: 0, other: 0, pos: p, dir: -v.norm(), mag: 40.0, src: 0 });
            println!("  crash forçado no tick {}", tick);
        }
        for &(t, bit) in &args.inject {
            if t == tick {
                world.get_mut::<InputSource>().pressed_bits |= bit;
                println!("  falha injetada no tick {}: {:#x}", tick, bit);
            }
        }
        let ts = Instant::now();
        sim.run(world, pool, TickInfo { tick, dt: DT, sub: 0, alpha: 0.0 }, opts);
        sim_walls.push(ts.elapsed().as_secs_f64() * 1e3);
        let shoot = args.shots.contains(&tick);
        if tick % 4 == 0 || shoot {
            render.run(world, pool, TickInfo { tick, dt: DT, sub: 0, alpha: 1.0 }, opts);
            renders += 1;
        }
        prof.end_frame(1, 1.0);
        crash::heartbeat(tick + 1);
        if shoot {
            screenshot(world, sim, render, &mut editor, tick);
        }
    }
    let wall = t.elapsed().as_secs_f64();
    // mediana: um travamento ou um relatório no meio da corrida não distorce a medida
    sim_walls.sort_by(f64::total_cmp);
    let sim_avg_ms = sim_walls.get(sim_walls.len() / 2).copied().unwrap_or(0.0);
    let span_ms = analysis::critical_path(sim, &analysis::avg_durations(sim)).span;
    let race = world.get::<RaceState>().clone();
    let crashes = world.get::<CrashState>().clone();
    let parts = world.get::<Particles>().pos.len();
    println!("Tempo real: {:.2} s ({} ticks + {} frames renderizados)", wall, ticks, renders);
    println!(
        "Grafo tick: {:.3} ms de parede por tick (mediana) vs caminho crítico {:.3} ms -> {:.3} ms são custo de agendamento",
        sim_avg_ms,
        span_ms,
        (sim_avg_ms - span_ms).max(0.0)
    );
    for car in 0..CARS {
        println!(
            "  {:<6} volta {} pos {}  s {:6.0} m  crashes {}  melhor volta {:.1} s{}",
            CAR_NAMES[car],
            race.lap[car],
            race.position[car],
            race.s[car],
            crashes.c[car].count,
            race.best[car],
            if race.finished[car] { "  (terminou)" } else { "" }
        );
    }
    println!("  partículas vivas: {}   drift score do jogador: {:.0}   near misses: {}", parts, race.drift_score, race.near_misses);
    let (replaying, checked, divergence) = world.with::<InputSource, _>(|s| (args.replay.is_some(), s.checked, s.divergence));
    if replaying {
        match divergence {
            None => println!("  replay determinístico: {} checkpoints de hash conferidos, nenhuma divergência", checked),
            Some(t) => println!("  replay DIVERGIU no tick {} ({} checkpoints conferidos)", t, checked),
        }
    }
    let fails = crash::failures();
    if !fails.is_empty() {
        println!("  falhas recuperadas nesta execução:");
        for f in fails {
            println!("    {}", f);
        }
    }

    let durs = vec![analysis::avg_durations(sim), analysis::avg_durations(render)];
    let header = format!(
        "Gerado por `corrida --analyze {}`: {} ticks de simulação, {} frames renderizados, {} lanes.\n\n{}",
        ticks,
        ticks,
        renders,
        pool.lanes,
        cook
    );
    let md = analysis::report_markdown(world, &[sim, render], &durs, &header);
    let _ = std::fs::create_dir_all("reports");
    let path = PathBuf::from("reports").join("analysis.md");
    match std::fs::write(&path, md) {
        Ok(()) => println!("Relatório: {}", path.display()),
        Err(e) => eprintln!("falha ao gravar {}: {}", path.display(), e),
    }
    println!("\nSugestões:");
    for i in analysis::insights(world, &[sim, render], &durs) {
        println!("  - {}", i);
    }
    out.save(world, sim, render, &format!("corrida --analyze {}", ticks));
    if let Err(e) = crash::save_input_log(&PathBuf::from("reports").join("analyze_input.log")) {
        eprintln!("falha ao salvar log: {}", e);
    }
}

/// Captura headless: framebuffer + HUD + editor (texto via GDI offscreen) em PNG.
fn screenshot(world: &mut World, sim: &Schedule, render: &Schedule, editor: &mut Editor, tick: u64) {
    let mut px = world.get::<Framebuffer>().px.clone();
    let mut texts = Vec::new();
    {
        let mut canvas = Canvas::full(W, H, &mut px);
        let hud = world.with::<RenderSnapshot, _>(|s| s.hud);
        let replay = world.with::<InputSource, _>(|s| (s.replay.is_some(), s.replay_len, s.divergence, s.checked));
        let autopilot = world.with::<PlayerIntent, _>(|p| p.autopilot);
        if !full_panel(editor.view) {
            hud::draw(&mut canvas, &mut texts, world, &hud, replay, tick, editor.view == View::Off, autopilot);
        }
        editor.draw(&mut canvas, &mut texts, world, &[sim, render]);
    }
    forja::platform::render_texts_offscreen(W, H, &mut px, &texts);
    let path = PathBuf::from("reports").join(format!("shot_{}.png", tick));
    let _ = std::fs::create_dir_all("reports");
    match forja::png::write_png(&path, W, H, &px) {
        Ok(()) => println!("  captura: {}", path.display()),
        Err(e) => eprintln!("falha ao gravar {}: {}", path.display(), e),
    }
}

/// Visões do editor que ocupam a tela inteira escondem o HUD do jogo.
fn full_panel(v: View) -> bool {
    matches!(v, View::Matrix | View::Tables | View::Failures)
}
