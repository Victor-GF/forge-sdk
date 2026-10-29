//! O banco de dados do jogo. Cada struct é uma tabela; `register` declara layout,
//! tempo de vida, colunas e invariantes. Agrupado por tempo de vida.

use forja::math::{v2, V2};
use forja::world::{Lifetime, Table, TableDecl, World};

use crate::track::{StaticScene, Track};

pub const CARS: usize = 4; // 0 = jogador, 1..3 = rivais
pub const PARTICLE_CAP: usize = 12_000;
pub const LAPS: u32 = 3;

pub const CAR_COLORS: [u32; CARS] = [0xE11D48, 0x2563EB, 0xF59E0B, 0x10B981];
pub const CAR_NAMES: [&str; CARS] = ["Você", "Azul", "Ouro", "Verde"];

// bits de input gravados por tick (o log de replay é uma sequência destes)
pub const IN_UP: u32 = 1;
pub const IN_DOWN: u32 = 2;
pub const IN_LEFT: u32 = 4;
pub const IN_RIGHT: u32 = 8;
pub const IN_HAND: u32 = 16;
pub const IN_BOOST: u32 = 32;
pub const IN_AUTO: u32 = 64;
// falhas injetadas também vão para o log: o replay reproduz o bug no mesmo tick
pub const F_NAN: u32 = 1 << 8;
pub const F_PANIC_VFX: u32 = 1 << 9;
pub const F_PANIC_RULES: u32 = 1 << 10;
pub const F_AV: u32 = 1 << 11;
pub const F_HANG: u32 = 1 << 12;
pub const FAULTS: u32 = F_NAN | F_PANIC_VFX | F_PANIC_RULES | F_AV | F_HANG;

macro_rules! table {
    ($t:ty, |$s:ident| $rows:expr) => {
        impl Table for $t {
            fn rows(&self) -> usize {
                let $s = self;
                $rows
            }
        }
    };
    ($t:ty, |$s:ident| $rows:expr, transient) => {
        impl Table for $t {
            fn rows(&self) -> usize {
                let $s = self;
                $rows
            }
            fn clear_transient(&mut self) {
                self.clear();
            }
        }
    };
}

// ------------------------------------------------------------------ sessão: veículos

/// Dados quentes: lidos por quase todo sistema. SoA.
#[derive(Clone, Default)]
pub struct VehicleHot {
    pub pos: Vec<V2>,
    pub vel: Vec<V2>,
    pub angle: Vec<f32>,
    pub ang_vel: Vec<f32>,
}
table!(VehicleHot, |s| s.pos.len());

/// Estado interno da simulação do carro: só física, VFX e regras olham. SoA.
#[derive(Clone, Default)]
pub struct VehicleSim {
    pub steer: Vec<f32>,
    pub slip: Vec<f32>,
    pub drift: Vec<f32>,
    pub track_idx: Vec<u32>,
    pub lateral: Vec<f32>,
    pub offroad: Vec<bool>,
    pub boost: Vec<f32>,
}
table!(VehicleSim, |s| s.steer.len());

#[derive(Clone, Copy, Default, Debug)]
pub struct Controls {
    pub throttle: f32,
    pub brake: f32,
    pub steer: f32,
    pub handbrake: bool,
    pub boost: bool,
}

/// Saída única de "quem dirige": jogador e IA escrevem a mesma coisa.
#[derive(Clone, Default)]
pub struct VehicleControl {
    pub c: Vec<Controls>,
}
table!(VehicleControl, |s| s.c.len());

#[derive(Clone, Default)]
pub struct PlayerIntent {
    pub c: Controls,
    pub autopilot: bool,
}
table!(PlayerIntent, |_s| 1);

#[derive(Clone, Copy, Default, Debug)]
pub struct Brain {
    pub target_lat: f32,
    pub target_speed: f32,
    pub look: f32,
    pub aggression: f32,
    pub avoid: f32,
    pub plan_tick: u32,
}

#[derive(Clone, Default)]
pub struct AIBrain {
    pub b: Vec<Brain>,
}
table!(AIBrain, |s| s.b.len());

pub const MODE_DRIVING: u8 = 0;
pub const MODE_CRASHING: u8 = 1;

/// Fria quase sempre, quente durante o crash. AoS de propósito: a física só lê `mode`,
/// e a análise aponta isso.
#[derive(Clone, Copy, Default, Debug)]
pub struct Crash {
    pub mode: u8,
    pub timer: f32,
    pub impact_speed: f32,
    pub impact_point: V2,
    pub normal: V2,
    pub invuln: f32,
    pub count: u32,
    pub seed: u32,
}

#[derive(Clone, Default)]
pub struct CrashState {
    pub c: Vec<Crash>,
}
table!(CrashState, |s| s.c.len());

pub const OBS_CONE: u8 = 0;
pub const OBS_BARREL: u8 = 1;
pub const OBS_BLOCK: u8 = 2;

/// AoS de 64 B com dados quentes e frios misturados: um caso para a análise de cache.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // home e label são dados frios de propósito
pub struct Obstacle {
    pub pos: V2,
    pub vel: V2,
    pub radius: f32,
    pub mass: f32,
    pub kind: u8,
    pub asleep: bool,
    pub color: u32,
    pub angle: f32,
    pub spin: f32,
    pub home: V2,
    pub hits: u32,
    pub label: [u8; 12],
}

#[derive(Clone, Default)]
pub struct Obstacles {
    pub o: Vec<Obstacle>,
}
table!(Obstacles, |s| s.o.len());

#[derive(Clone, Default)]
pub struct RaceState {
    pub lap: Vec<u32>,
    pub s: Vec<f32>,
    pub total: Vec<f32>,
    pub position: Vec<u8>,
    pub lap_start: Vec<f32>,
    pub best: Vec<f32>,
    pub finished: Vec<bool>,
    pub race_time: f32,
    pub drift_score: f32,
    pub combo: f32,
    pub combo_timer: f32,
    pub near_misses: u32,
    pub prev_player_pos: u8,
}
table!(RaceState, |s| s.lap.len());

// ------------------------------------------------------------------ transitórias (por tick)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventKind {
    Crash,
    Impact,
    NearMiss,
    Drift,
    Lap,
    Overtake,
    Wall,
}

pub const EVENT_KINDS: [EventKind; 7] = [EventKind::Crash, EventKind::Impact, EventKind::NearMiss, EventKind::Drift, EventKind::Lap, EventKind::Overtake, EventKind::Wall];

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub kind: EventKind,
    pub car: u8,
    pub other: u8,
    pub pos: V2,
    pub dir: V2,
    pub mag: f32,
    /// Sistema que produziu (para a visão produtor -> consumidor).
    pub src: u16,
}

/// O ponto de acoplamento do jogo: produtores escrevem, Diretor e VFX só leem.
#[derive(Clone, Default)]
pub struct Events {
    pub e: Vec<Event>,
}
impl Events {
    pub fn clear(&mut self) {
        self.e.clear();
    }
}
table!(Events, |s| s.e.len(), transient);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VfxKind {
    Sparks,
    Debris,
    Smoke,
    Ink,
    Ring,
    Confetti,
}

#[derive(Clone, Copy, Debug)]
pub struct VfxReq {
    pub kind: VfxKind,
    pub pos: V2,
    pub dir: V2,
    pub mag: f32,
    pub color: u32,
}

#[derive(Clone, Default)]
pub struct VfxRequests {
    pub r: Vec<VfxReq>,
}
impl VfxRequests {
    pub fn clear(&mut self) {
        self.r.clear();
    }
}
table!(VfxRequests, |s| s.r.len(), transient);

// ------------------------------------------------------------------ direção

pub const BANNER_NONE: u8 = 0;
pub const BANNER_CRASH: u8 = 1;
pub const BANNER_NEAR: u8 = 2;
pub const BANNER_LAP: u8 = 3;
pub const BANNER_OVERTAKE: u8 = 4;
pub const BANNER_DRIFT: u8 = 5;
pub const BANNER_FINISH: u8 = 6;

#[derive(Clone, Default)]
pub struct CameraRig {
    pub pos: V2,
    pub zoom: f32,
    pub angle: f32,
    pub shake: f32,
    pub shake_off: V2,
    pub cine: f32,
    pub cine_target: f32,
    pub focus: V2,
    pub flash: f32,
    pub speedlines: f32,
    pub near_cd: f32,
    pub ink_cd: f32,
    pub banner: u8,
    pub banner_t: f32,
    pub banner_val: f32,
    pub seed: u32,
}
table!(CameraRig, |_s| 1);

/// Pedido de escala de tempo: o loop principal lê (consumidor externo ao grafo).
#[derive(Clone)]
pub struct TimeControl {
    pub target: f32,
    pub hold: f32,
}
table!(TimeControl, |_s| 1);

// ------------------------------------------------------------------ VFX

pub const P_SPARK: u8 = 0;
pub const P_SMOKE: u8 = 1;
pub const P_DEBRIS: u8 = 2;
pub const P_INK: u8 = 3;
pub const P_RING: u8 = 4;
pub const P_CONFETTI: u8 = 5;
pub const P_DUST: u8 = 6;

#[derive(Clone, Default)]
pub struct Particles {
    pub pos: Vec<V2>,
    pub vel: Vec<V2>,
    pub life: Vec<f32>,
    pub max_life: Vec<f32>,
    pub size: Vec<f32>,
    pub color: Vec<u32>,
    pub kind: Vec<u8>,
    pub rot: Vec<f32>,
    pub dropped: u32,
}
table!(Particles, |s| s.pos.len());

impl Particles {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(&mut self, pos: V2, vel: V2, life: f32, size: f32, color: u32, kind: u8, rot: f32) {
        if self.pos.len() >= PARTICLE_CAP {
            self.dropped += 1;
            return;
        }
        self.pos.push(pos);
        self.vel.push(vel);
        self.life.push(life);
        self.max_life.push(life);
        self.size.push(size);
        self.color.push(color);
        self.kind.push(kind);
        self.rot.push(rot);
    }

    pub fn swap_remove(&mut self, i: usize) {
        self.pos.swap_remove(i);
        self.vel.swap_remove(i);
        self.life.swap_remove(i);
        self.max_life.swap_remove(i);
        self.size.swap_remove(i);
        self.color.swap_remove(i);
        self.kind.swap_remove(i);
        self.rot.swap_remove(i);
    }
}

// ------------------------------------------------------------------ fronteira com o render

#[derive(Clone, Copy, Default, Debug)]
pub struct CarSnap {
    pub pos: V2,
    pub angle: f32,
    pub color: u32,
    pub brake: f32,
    pub crashing: bool,
    pub boost: bool,
    pub ghost: bool,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct ObsSnap {
    pub pos: V2,
    pub radius: f32,
    pub kind: u8,
    pub color: u32,
    pub angle: f32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct PartSnap {
    pub pos: V2,
    pub vel: V2,
    pub t: f32,
    pub size: f32,
    pub color: u32,
    pub kind: u8,
    pub rot: f32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct CamSnap {
    pub pos: V2,
    pub zoom: f32,
    pub angle: f32,
    pub cine: f32,
    pub flash: f32,
    pub speedlines: f32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Hud {
    pub speed: f32,
    pub lap: u32,
    pub position: u8,
    pub drift_score: f32,
    pub combo: f32,
    pub race_time: f32,
    pub best: f32,
    pub banner: u8,
    pub banner_t: f32,
    pub banner_val: f32,
    pub boost: f32,
    pub finished: bool,
}

/// Cópia imutável do que o render precisa, feita no fim do tick. O render só lê isto,
/// então poderia rodar em paralelo com o tick seguinte (pipelining).
#[derive(Clone, Default)]
pub struct RenderSnapshot {
    pub cars: Vec<CarSnap>,
    pub cars_prev: Vec<CarSnap>,
    pub obstacles: Vec<ObsSnap>,
    pub parts: Vec<PartSnap>,
    pub cam: CamSnap,
    pub cam_prev: CamSnap,
    pub hud: Hud,
    pub valid: bool,
}
table!(RenderSnapshot, |s| s.parts.len() + s.cars.len() * 2 + s.obstacles.len());

#[derive(Clone)]
pub struct Framebuffer {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}
table!(Framebuffer, |s| s.px.len());

// ------------------------------------------------------------------ input / depuração

#[derive(Clone, Default)]
pub struct InputSource {
    pub live_bits: u32,
    pub pressed_bits: u32,
    pub replay: Option<forja::crash::InputLog>,
    pub replay_len: usize,
    pub divergence: Option<u64>,
    pub checked: u32,
}
table!(InputSource, |_s| 1);

#[derive(Clone, Default)]
pub struct DebugFaults {
    pub bits: u32,
}
impl DebugFaults {
    pub fn clear(&mut self) {
        self.bits = 0;
    }
}
table!(DebugFaults, |_s| 1, transient);

// ------------------------------------------------------------------ registro

pub fn register(world: &mut World, track: Track, scene: StaticScene, fb_w: usize, fb_h: usize) {
    // estáticas (cozidas na carga)
    world.add(
        TableDecl::soa("Track", Lifetime::Static).col("pos", 8).col("tan", 8).col("s", 4).col("curv", 4).sparse().note("spline cozida; consulta local"),
        track,
    );
    world.add(TableDecl::aos::<u32>("StaticScene", Lifetime::Static).col("rgb", 4).sparse().note("cenário cozido (iluminação baked)"), scene);

    // sessão: veículos
    let starts = crate::track::grid_positions();
    let hot = VehicleHot {
        pos: starts.iter().map(|g| g.0).collect(),
        vel: vec![V2::ZERO; CARS],
        angle: starts.iter().map(|g| g.1).collect(),
        ang_vel: vec![0.0; CARS],
    };
    world.add(TableDecl::soa("VehicleHot", Lifetime::Session).col("pos", 8).col("vel", 8).col("angle", 4).col("ang_vel", 4).note("lida por quase todos"), hot);
    world.add(
        TableDecl::soa("VehicleSim", Lifetime::Session).col("steer", 4).col("slip", 4).col("drift", 4).col("track_idx", 4).col("lateral", 4).col("offroad", 1).col("boost", 4),
        VehicleSim {
            steer: vec![0.0; CARS],
            slip: vec![0.0; CARS],
            drift: vec![0.0; CARS],
            track_idx: starts.iter().map(|g| g.2).collect(),
            lateral: vec![0.0; CARS],
            offroad: vec![false; CARS],
            boost: vec![1.0; CARS],
        },
    );
    world.add(
        TableDecl::aos::<Controls>("VehicleControl", Lifetime::Session).col("throttle", 4).col("brake", 4).col("steer", 4).col("handbrake", 1).col("boost", 1).note("jogador OU IA"),
        VehicleControl { c: vec![Controls::default(); CARS] },
    );
    world.add(TableDecl::aos::<PlayerIntent>("PlayerIntent", Lifetime::Session).col("controls", 14).col("autopilot", 1), PlayerIntent::default());
    let brains = (0..CARS)
        .map(|i| Brain { aggression: [0.92, 0.97, 1.0, 0.95][i], look: 20.0, target_speed: 30.0, ..Default::default() })
        .collect();
    world.add(
        TableDecl::aos::<Brain>("AIBrain", Lifetime::Session).col("target_lat", 4).col("target_speed", 4).col("look", 4).col("aggression", 4).col("avoid", 4).col("plan_tick", 4),
        AIBrain { b: brains },
    );
    world.add(
        TableDecl::aos::<Crash>("CrashState", Lifetime::Session)
            .col("mode", 1)
            .col("timer", 4)
            .col("impact_speed", 4)
            .col("impact_point", 8)
            .col("normal", 8)
            .col("invuln", 4)
            .col("count", 4)
            .col("seed", 4)
            .note("fria; quente no crash"),
        CrashState { c: vec![Crash::default(); CARS] },
    );
    world.add(
        TableDecl::aos::<Obstacle>("Obstacles", Lifetime::Session)
            .col("pos", 8)
            .col("vel", 8)
            .col("radius", 4)
            .col("mass", 4)
            .col("kind", 1)
            .col("asleep", 1)
            .col("color", 4)
            .col("angle", 4)
            .col("spin", 4)
            .col("home", 8)
            .col("hits", 4)
            .col("label", 12)
            .note("quente+frio misturados"),
        Obstacles { o: crate::track::obstacle_layout() },
    );
    world.add(
        TableDecl::soa("RaceState", Lifetime::Session).col("lap", 4).col("s", 4).col("total", 4).col("position", 1).col("lap_start", 4).col("best", 4).col("finished", 1),
        RaceState {
            lap: vec![0; CARS],
            s: starts.iter().map(|g| g.3).collect(),
            total: vec![0.0; CARS],
            position: (1..=CARS as u8).collect(),
            lap_start: vec![0.0; CARS],
            best: vec![0.0; CARS],
            finished: vec![false; CARS],
            prev_player_pos: 1,
            ..Default::default()
        },
    );

    // transitórias
    world.add(
        TableDecl::aos::<Event>("Events", Lifetime::Transient).col("kind", 1).col("car", 1).col("other", 1).col("pos", 8).col("dir", 8).col("mag", 4).col("src", 2).note("acoplamento do jogo"),
        Events::default(),
    );
    world.add(TableDecl::aos::<VfxReq>("VfxRequests", Lifetime::Transient).col("kind", 1).col("pos", 8).col("dir", 8).col("mag", 4).col("color", 4), VfxRequests::default());

    // direção
    world.add(
        TableDecl::aos::<CameraRig>("CameraRig", Lifetime::Session).col("pos", 8).col("zoom", 4).col("angle", 4).col("shake", 12).col("cine", 16).col("fx", 12).col("director", 24),
        CameraRig { pos: hot_player_start(&starts), zoom: 10.0, ..Default::default() },
    );
    world.add(TableDecl::aos::<TimeControl>("TimeControl", Lifetime::Session).col("target", 4).col("hold", 4).external().note("lido pelo loop principal"), TimeControl { target: 1.0, hold: 0.0 });

    world.add(
        TableDecl::soa("Particles", Lifetime::Session).col("pos", 8).col("vel", 8).col("life", 4).col("max_life", 4).col("size", 4).col("color", 4).col("kind", 1).col("rot", 4).note("pool SoA"),
        Particles::default(),
    );
    world.add(TableDecl::aos::<PartSnap>("RenderSnapshot", Lifetime::Session).col("snap", std::mem::size_of::<PartSnap>()).note("cópia imutável p/ render"), RenderSnapshot::default());
    world.add(
        TableDecl::aos::<u32>("Framebuffer", Lifetime::Session).col("bgra", 4).sparse().external().note("lido pela plataforma"),
        Framebuffer { w: fb_w, h: fb_h, px: vec![0; fb_w * fb_h] },
    );
    world.add(TableDecl::aos::<InputSource>("InputSource", Lifetime::Session).col("bits", 8).col("replay", 32), InputSource::default());
    world.add(TableDecl::aos::<DebugFaults>("DebugFaults", Lifetime::Transient).col("bits", 4).note("falhas injetadas (F5-F9)"), DebugFaults::default());

    // invariantes: o equivalente a constraints de SQL, checadas depois de cada escrita
    world.invariant::<VehicleHot>("finito", |t| {
        for i in 0..t.pos.len() {
            if !t.pos[i].is_finite() || !t.vel[i].is_finite() || !t.angle[i].is_finite() || !t.ang_vel[i].is_finite() {
                return Err(format!("carro {} ({}) com valor não finito: pos {:?} vel {:?}", i, CAR_NAMES[i], t.pos[i], t.vel[i]));
            }
            if t.vel[i].len() > 150.0 {
                return Err(format!("carro {} acima de 150 m/s: {:.1}", i, t.vel[i].len()));
            }
        }
        Ok(())
    });
    world.invariant::<Particles>("pool consistente", |p| {
        let n = p.pos.len();
        if [p.vel.len(), p.life.len(), p.max_life.len(), p.size.len(), p.color.len(), p.kind.len(), p.rot.len()].iter().any(|&l| l != n) {
            return Err("colunas do pool com tamanhos diferentes".into());
        }
        if n > PARTICLE_CAP {
            return Err(format!("pool com {} partículas, capacidade {}", n, PARTICLE_CAP));
        }
        Ok(())
    });
    world.invariant::<RaceState>("posições válidas", |r| {
        let mut seen = [false; CARS];
        for &p in &r.position {
            if p == 0 || p as usize > CARS || seen[p as usize - 1] {
                return Err(format!("posições inválidas: {:?}", r.position));
            }
            seen[p as usize - 1] = true;
        }
        Ok(())
    });
    world.invariant::<Obstacles>("finito", |o| {
        for (i, ob) in o.o.iter().enumerate() {
            if !ob.pos.is_finite() || !ob.vel.is_finite() {
                return Err(format!("obstáculo {} com posição não finita", i));
            }
        }
        Ok(())
    });
}

fn hot_player_start(starts: &[(V2, f32, u32, f32)]) -> V2 {
    starts.first().map(|s| s.0).unwrap_or(v2(0.0, 0.0))
}
