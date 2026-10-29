//! Sistemas de simulação (grafo "tick", 60 Hz fixo). Cada função faz seu trabalho
//! inteiro, de forma monolítica; o que a torna analisável é a declaração em `sim_systems`.

use std::time::{Duration, Instant};

use forja::math::{lerp, v2, wrap_angle, Rng, V2};
use forja::schedule::{SysCtx, SysFn};

use crate::tables::*;
use crate::track::{Track, HALF_WIDTH, WALL};

/// Funções de sistema disponíveis, pelo nome. Quem roda, em que fase e ordem, lendo e
/// escrevendo o quê, é decidido em `projeto.forja`; aqui só se liga o nome à implementação.
pub fn register(fns: &mut Vec<(&'static str, SysFn)>) {
    fns.extend_from_slice(&[
        ("Input.Sample", input_sample as SysFn),
        ("AI.Decide", ai_decide),
        ("Control.Merge", control_merge),
        ("Vehicle.Physics", vehicle_physics),
        ("Obstacles.Simulate", obstacles_sim),
        ("Collision.Resolve", collision),
        ("Crash.Update", crash_update),
        ("Race.Rules", race_rules),
        ("Race.Positions", race_positions),
        ("Director", director),
        ("VFX.Simulate", vfx_simulate),
        ("Render.Extract", render_extract),
        ("Determinism.Hash", determinism_hash),
    ]);
}

fn event(kind: EventKind, car: usize, other: usize, pos: V2, dir: V2, mag: f32, src: u16) -> Event {
    Event { kind, car: car as u8, other: other as u8, pos, dir, mag, src }
}

// ------------------------------------------------------------------ input

fn input_sample(ctx: &mut SysCtx) {
    let tick = ctx.tick();
    let mut src = ctx.write::<InputSource>();
    let mut intent = ctx.write::<PlayerIntent>();
    let mut faults = ctx.write::<DebugFaults>();
    let live = src.live_bits | src.pressed_bits;
    let bits = match &src.replay {
        Some(log) if (tick as usize) < log.inputs.len() => log.inputs[tick as usize],
        Some(_) => {
            src.replay = None;
            forja::crash::breadcrumb(format!("replay terminou no tick {}: controle devolvido ao jogador", tick));
            live
        }
        None => live,
    };
    src.pressed_bits = 0;
    forja::crash::set_tick(tick);
    forja::crash::record_input(tick, bits);

    if bits & IN_AUTO != 0 {
        intent.autopilot = !intent.autopilot;
    }
    let b = |m: u32| if bits & m != 0 { 1.0 } else { 0.0 };
    intent.c = Controls {
        throttle: b(IN_UP),
        brake: b(IN_DOWN),
        steer: b(IN_RIGHT) - b(IN_LEFT),
        handbrake: bits & IN_HAND != 0,
        boost: bits & IN_BOOST != 0,
    };
    faults.bits = bits & FAULTS;
}

// ------------------------------------------------------------------ IA

fn ai_decide(ctx: &mut SysCtx) {
    if ctx.read::<DebugFaults>().bits & F_HANG != 0 {
        // Travamento proposital (F9): o watchdog deve flagrar este sistema.
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(6) {
            std::hint::spin_loop();
        }
    }
    let hot = ctx.read::<VehicleHot>();
    let sim = ctx.read::<VehicleSim>();
    let track = ctx.read::<Track>();
    let obs = ctx.read::<Obstacles>();
    let race = ctx.read::<RaceState>();
    let mut brains = ctx.write::<AIBrain>();
    let tick = ctx.tick();

    for car in 0..CARS {
        if (car as u64 + tick) % 3 != 0 {
            continue; // time slicing
        }
        let b = &mut brains.b[car];
        let speed = hot.vel[car].len();
        let idx = sim.track_idx[car] as usize;
        b.look = 10.0 + speed * 0.55;
        let curv = track.curvature_ahead(idx, 25.0 + speed * 1.7);
        let vmax_curve = (24.0 * b.aggression / curv.abs().max(1e-4)).sqrt();
        b.target_speed = vmax_curve.clamp(22.0, 80.0);
        // traçado: por dentro da próxima curva
        let lat = curv.signum() * (curv.abs() * 900.0).min(5.0);

        let fwd = track.tan[idx];
        let nrm = track.normal(idx);
        let mut avoid = 0.0;
        for o in &obs.o {
            let rel = o.pos - hot.pos[car];
            let ahead = rel.dot(fwd);
            if !(2.0..=45.0).contains(&ahead) {
                continue;
            }
            let olat = (o.pos - track.pos[idx]).dot(nrm);
            let gap = olat - lat;
            let need = o.radius + 2.8;
            if gap.abs() < need {
                let w = match o.kind {
                    OBS_BLOCK => 1.5,
                    OBS_BARREL => 1.0,
                    _ => 0.45,
                };
                avoid += if gap > 0.0 { -1.0 } else { 1.0 } * (need - gap.abs() + 0.5) * w;
            }
        }
        for other in 0..CARS {
            if other == car {
                continue;
            }
            let ahead = (hot.pos[other] - hot.pos[car]).dot(fwd);
            if (3.0..25.0).contains(&ahead) && (sim.lateral[other] - lat).abs() < 3.5 {
                avoid += if sim.lateral[other] > lat { -3.0 } else { 3.0 };
            }
        }
        b.avoid = avoid.clamp(-7.0, 7.0);
        b.target_lat = (lat + b.avoid).clamp(-HALF_WIDTH + 2.0, HALF_WIDTH - 2.0);
        if car != 0 {
            // rubber band leve em relação ao jogador
            let gap = race.total[0] - race.total[car];
            b.target_speed *= 1.0 + (gap / 400.0).clamp(-0.12, 0.10);
        }
        b.plan_tick = tick as u32;
    }
}

fn control_merge(ctx: &mut SysCtx) {
    let intent = ctx.read::<PlayerIntent>();
    let brains = ctx.read::<AIBrain>();
    let hot = ctx.read::<VehicleHot>();
    let sim = ctx.read::<VehicleSim>();
    let track = ctx.read::<Track>();
    let mut ctl = ctx.write::<VehicleControl>();
    for car in 0..CARS {
        if car == 0 && !intent.autopilot {
            ctl.c[0] = intent.c;
            continue;
        }
        let b = &brains.b[car];
        let idx = sim.track_idx[car] as usize;
        let fwd = V2::from_angle(hot.angle[car]);
        let speed = hot.vel[car].dot(fwd);
        let ti = track.idx_at_s(track.s[idx] + b.look);
        let target = track.pos[ti] + track.normal(ti) * b.target_lat;
        let to = target - hot.pos[car];
        let local = wrap_angle(to.y.atan2(to.x) - hot.angle[car]);
        let steer = (local * 2.4).clamp(-1.0, 1.0);
        let (throttle, brake) = if speed < b.target_speed {
            (1.0, 0.0)
        } else if speed > b.target_speed + 3.0 {
            (0.0, ((speed - b.target_speed) / 10.0).clamp(0.0, 1.0))
        } else {
            (0.3, 0.0)
        };
        let boost = b.aggression > 0.95 && local.abs() < 0.05 && speed > 40.0 && speed < b.target_speed - 5.0;
        ctl.c[car] = Controls { throttle, brake, steer, handbrake: false, boost };
    }
}

// ------------------------------------------------------------------ física

fn vehicle_physics(ctx: &mut SysCtx) {
    const SUB: usize = 4;
    let dt = ctx.dt();
    let h = dt / SUB as f32;
    let src = ctx.id();
    let ctl = ctx.read::<VehicleControl>();
    let track = ctx.read::<Track>();
    let crash = ctx.read::<CrashState>();
    let faults = ctx.read::<DebugFaults>();
    let mut hotw = ctx.write::<VehicleHot>();
    let mut simw = ctx.write::<VehicleSim>();
    let mut ev = ctx.write::<Events>();
    let hot = &mut *hotw;
    let sim = &mut *simw;

    for car in 0..CARS {
        if crash.c[car].mode == MODE_CRASHING {
            // balístico: o carro desliza girando até parar
            for _ in 0..SUB {
                hot.vel[car] *= 1.0 - 1.4 * h;
                hot.ang_vel[car] *= 1.0 - 1.1 * h;
                hot.pos[car] += hot.vel[car] * h;
                hot.angle[car] += hot.ang_vel[car] * h;
            }
        } else {
            let c = ctl.c[car];
            let grip = if sim.offroad[car] { 0.55 } else { 1.0 };
            for _ in 0..SUB {
                let f = V2::from_angle(hot.angle[car]);
                let r = f.perp();
                let mut vf = hot.vel[car].dot(f);
                let mut vr = hot.vel[car].dot(r);
                let boosting = c.boost && sim.boost[car] > 0.0;
                let vmax = if boosting { 95.0 } else { 80.0 };
                let mut acc = c.throttle * 32.0 * (1.0 - vf / vmax).max(0.0) * if boosting { 1.5 } else { 1.0 };
                if c.brake > 0.0 {
                    if vf > 0.5 {
                        acc -= c.brake * 48.0;
                    } else if vf > -12.0 {
                        acc -= c.brake * 12.0;
                    }
                }
                acc -= vf * (0.004 * vf.abs() + if sim.offroad[car] { 0.9 } else { 0.06 });
                vf += acc * h;

                let lock = 0.6 / (1.0 + vf.abs() / 28.0);
                sim.steer[car] += (c.steer * lock - sim.steer[car]) * (8.0 * h).min(1.0);
                let yaw_target = vf * sim.steer[car].tan() / 2.7;
                let hb = c.handbrake && vf.abs() > 8.0;
                let lat_grip = if hb { 1.2 } else { 11.0 } * grip;
                vr *= 1.0 / (1.0 + lat_grip * h);
                let yaw_resp = if hb { 2.5 } else { 9.0 } * grip;
                let drift_gain = if hb { 1.5 } else { 1.0 + sim.drift[car] * 0.4 };
                hot.ang_vel[car] += (yaw_target * drift_gain - hot.ang_vel[car]) * (yaw_resp * h).min(1.0);

                hot.vel[car] = f * vf + r * vr;
                hot.pos[car] += hot.vel[car] * h;
                hot.angle[car] = wrap_angle(hot.angle[car] + hot.ang_vel[car] * h);

                if boosting {
                    sim.boost[car] = (sim.boost[car] - h * 0.25).max(0.0);
                } else {
                    sim.boost[car] = (sim.boost[car] + h * 0.05).min(1.0);
                }
                let speed = hot.vel[car].len();
                sim.slip[car] = vr.abs() / speed.max(1.0);
                let target = ((sim.slip[car] - 0.12).max(0.0) * 3.0).min(1.0);
                sim.drift[car] += (target - sim.drift[car]) * (6.0 * h).min(1.0);
            }
        }
        let (idx, lat) = track.nearest(hot.pos[car], sim.track_idx[car]);
        sim.track_idx[car] = idx as u32;
        sim.lateral[car] = lat;
        sim.offroad[car] = lat.abs() > HALF_WIDTH + 0.8;
        let speed = hot.vel[car].len();
        if crash.c[car].mode == MODE_DRIVING && sim.drift[car] > 0.3 && speed > 18.0 {
            let back = hot.pos[car] - V2::from_angle(hot.angle[car]) * 2.0;
            ev.e.push(event(EventKind::Drift, car, car, back, hot.vel[car].norm(), sim.drift[car] * speed, src));
        }
    }
    if faults.bits & F_NAN != 0 {
        // F5: um NaN "escapa" da integração. A invariante de VehicleHot pega, e a política
        // SkipTick desfaz o tick inteiro desta tabela.
        hot.vel[2] = v2(f32::NAN, f32::NAN);
    }
}

fn obstacles_sim(ctx: &mut SysCtx) {
    let dt = ctx.dt();
    let mut obs = ctx.write::<Obstacles>();
    for o in obs.o.iter_mut() {
        if o.asleep || o.kind == OBS_BLOCK {
            continue;
        }
        o.pos += o.vel * dt;
        o.vel *= (1.0 - 2.5 * dt).max(0.0);
        o.angle += o.spin * dt;
        o.spin *= (1.0 - 2.0 * dt).max(0.0);
        if o.vel.len2() < 0.01 && o.spin.abs() < 0.1 {
            o.asleep = true;
            o.vel = V2::ZERO;
        }
    }
}

fn circles(p: V2, a: f32) -> [V2; 2] {
    let f = V2::from_angle(a);
    [p + f * 1.25, p - f * 1.25]
}
const CAR_R: f32 = 1.1;

fn collision(ctx: &mut SysCtx) {
    let dt = ctx.dt();
    let src = ctx.id();
    if ctx.read::<DebugFaults>().bits & F_AV != 0 {
        // F8: falha nativa proposital (escrita num endereço inválido). Não é um panic:
        // catch_unwind não pega, o filtro SEH grava minidump + relatório.
        unsafe { std::ptr::write_volatile(0x10 as *mut u32, 0xDEAD) };
    }
    let track = ctx.read::<Track>();
    let sim = ctx.read::<VehicleSim>();
    let crash = ctx.read::<CrashState>();
    let mut hotw = ctx.write::<VehicleHot>();
    let mut obsw = ctx.write::<Obstacles>();
    let mut ev = ctx.write::<Events>();
    let hot = &mut *hotw;
    let obs = &mut *obsw;
    let driving = |car: usize| crash.c[car].mode == MODE_DRIVING;

    // muros de pneus (derivados da pista)
    for car in 0..CARS {
        let lat = sim.lateral[car];
        let limit = WALL - 1.6;
        if lat.abs() > limit {
            let n = track.normal(sim.track_idx[car] as usize) * lat.signum();
            hot.pos[car] -= n * (lat.abs() - limit);
            let vn = hot.vel[car].dot(n);
            if vn > 0.0 {
                hot.vel[car] -= n * vn * 1.35;
                hot.vel[car] *= 0.92;
                let p = hot.pos[car] + n * 1.0;
                if driving(car) && vn > 22.0 {
                    ev.e.push(event(EventKind::Crash, car, car, p, -n, vn, src));
                } else if vn > 4.0 {
                    ev.e.push(event(EventKind::Wall, car, car, p, -n, vn, src));
                }
            }
        }
    }

    // carro x carro
    for a in 0..CARS {
        for b in a + 1..CARS {
            if crash.c[a].invuln > 0.0 || crash.c[b].invuln > 0.0 {
                continue;
            }
            let ca = circles(hot.pos[a], hot.angle[a]);
            let cb = circles(hot.pos[b], hot.angle[b]);
            let mut best: Option<(f32, V2, V2)> = None;
            for pa in ca {
                for pb in cb {
                    let d = pb - pa;
                    let dist = d.len();
                    if dist < 2.0 * CAR_R && dist > 1e-4 {
                        let pen = 2.0 * CAR_R - dist;
                        if best.is_none_or(|(bp, _, _)| pen > bp) {
                            best = Some((pen, d / dist, pa + d * 0.5));
                        }
                    }
                }
            }
            if let Some((pen, n, point)) = best {
                hot.pos[a] -= n * (pen * 0.5);
                hot.pos[b] += n * (pen * 0.5);
                let rel = (hot.vel[a] - hot.vel[b]).dot(n);
                if rel > 0.0 {
                    let j = rel * 1.3 * 0.5;
                    hot.vel[a] -= n * j;
                    hot.vel[b] += n * j;
                    // batida fora do centro gira o carro
                    let fa = V2::from_angle(hot.angle[a]);
                    hot.angle[a] += (fa.cross(n) * rel * 0.004).clamp(-0.08, 0.08);
                    if rel > 20.0 {
                        if driving(a) {
                            ev.e.push(event(EventKind::Crash, a, b, point, -n, rel, src));
                        }
                        if driving(b) {
                            ev.e.push(event(EventKind::Crash, b, a, point, n, rel, src));
                        }
                    } else if rel > 3.0 {
                        ev.e.push(event(EventKind::Impact, a, b, point, -n, rel, src));
                    }
                }
            } else if a == 0 || b == 0 {
                // near miss: entrou agora na zona de 6.5 m em alta velocidade relativa
                let d = (hot.pos[b] - hot.pos[a]).len();
                let prev = ((hot.pos[b] - hot.vel[b] * dt) - (hot.pos[a] - hot.vel[a] * dt)).len();
                let relv = (hot.vel[a] - hot.vel[b]).len();
                if d < 6.5 && prev >= 6.5 && relv > 14.0 && driving(a) && driving(b) {
                    let other = if a == 0 { b } else { a };
                    ev.e.push(event(EventKind::NearMiss, 0, other, hot.pos[a].lerp(hot.pos[b], 0.5), hot.vel[0].norm(), relv, src));
                }
            }
        }
    }

    // carro x obstáculos
    for car in 0..CARS {
        let cc = circles(hot.pos[car], hot.angle[car]);
        for o in obs.o.iter_mut() {
            if (o.pos - hot.pos[car]).len2() > 36.0 {
                continue;
            }
            for c in cc {
                let d = o.pos - c;
                let dist = d.len();
                let min = CAR_R + o.radius;
                if dist >= min {
                    continue;
                }
                let n = if dist > 1e-4 { d / dist } else { V2::from_angle(hot.angle[car]) };
                let vn = hot.vel[car].dot(n);
                if o.kind == OBS_BLOCK {
                    hot.pos[car] -= n * (min - dist);
                    if vn > 0.0 {
                        hot.vel[car] -= n * vn * 1.3;
                        if driving(car) && vn > 18.0 {
                            ev.e.push(event(EventKind::Crash, car, car, o.pos - n * o.radius, -n, vn, src));
                        } else if vn > 3.0 {
                            ev.e.push(event(EventKind::Impact, car, car, o.pos - n * o.radius, -n, vn, src));
                        }
                    }
                } else {
                    o.pos += n * (min - dist);
                    o.vel = hot.vel[car] * 1.15 + n * (4.0 + vn.max(0.0) * 0.4);
                    o.spin = (vn + 2.0) * if n.cross(hot.vel[car]) > 0.0 { 0.9 } else { -0.9 };
                    o.asleep = false;
                    o.hits += 1;
                    let ratio = o.mass / (o.mass + 1200.0);
                    hot.vel[car] -= n * (vn.max(0.0) * ratio * 2.0);
                    if vn > 6.0 {
                        let m = if o.kind == OBS_CONE { vn * 0.4 } else { vn * 0.8 };
                        ev.e.push(event(EventKind::Impact, car, car, o.pos, n, m, src));
                    }
                }
                break;
            }
        }
    }
}

fn crash_update(ctx: &mut SysCtx) {
    let dt = ctx.dt();
    let tick = ctx.tick();
    let track = ctx.read::<Track>();
    let sim = ctx.read::<VehicleSim>();
    let ev = ctx.read::<Events>();
    let mut crash = ctx.write::<CrashState>();
    let mut hot = ctx.write::<VehicleHot>();

    for e in ev.e.iter().filter(|e| e.kind == EventKind::Crash) {
        let car = e.car as usize;
        let c = &mut crash.c[car];
        if c.mode != MODE_DRIVING || c.invuln > 0.0 {
            continue;
        }
        c.mode = MODE_CRASHING;
        c.timer = 2.0;
        c.impact_speed = e.mag;
        c.impact_point = e.pos;
        c.normal = e.dir;
        c.count += 1;
        c.seed = (tick as u32) ^ (car as u32 * 7919);
        let spin = if c.seed & 1 == 0 { 1.0 } else { -1.0 };
        hot.ang_vel[car] = spin * (e.mag * 0.3).min(12.0);
        hot.vel[car] = hot.vel[car] * 0.7 + e.dir * (e.mag * 0.25);
        forja::crash::breadcrumb(format!("crash: {} a {:.0} km/h", CAR_NAMES[car], e.mag * 3.6));
    }
    for car in 0..CARS {
        let c = &mut crash.c[car];
        if c.invuln > 0.0 {
            c.invuln -= dt;
        }
        if c.mode == MODE_CRASHING {
            c.timer -= dt;
            if c.timer <= 0.0 {
                let idx = track.idx_at_s(track.s[sim.track_idx[car] as usize] - 12.0);
                hot.pos[car] = track.pos[idx];
                hot.angle[car] = track.tan[idx].y.atan2(track.tan[idx].x);
                hot.vel[car] = track.tan[idx] * 15.0;
                hot.ang_vel[car] = 0.0;
                c.mode = MODE_DRIVING;
                c.invuln = 2.0;
            }
        }
    }
}

// ------------------------------------------------------------------ regras

fn race_rules(ctx: &mut SysCtx) {
    if ctx.read::<DebugFaults>().bits & F_PANIC_RULES != 0 {
        panic!("RaceState corrompido: volta negativa detectada (falha injetada com F7)");
    }
    let dt = ctx.dt();
    let src = ctx.id();
    let track = ctx.read::<Track>();
    let sim = ctx.read::<VehicleSim>();
    let mut racew = ctx.write::<RaceState>();
    let mut ev = ctx.write::<Events>();
    let race = &mut *racew;
    let len = track.length;
    race.race_time += dt;
    let mut new_events = Vec::new();
    for car in 0..CARS {
        let s_new = track.s[sim.track_idx[car] as usize];
        let prev = race.s[car];
        if prev > len * 0.75 && s_new < len * 0.25 {
            race.lap[car] += 1;
            if race.lap[car] >= 2 {
                let lt = race.race_time - race.lap_start[car];
                race.best[car] = if race.best[car] == 0.0 { lt } else { race.best[car].min(lt) };
            }
            race.lap_start[car] = race.race_time;
            if race.lap[car] > LAPS && !race.finished[car] {
                race.finished[car] = true;
                forja::crash::breadcrumb(format!("{} terminou a corrida", CAR_NAMES[car]));
            }
            new_events.push(event(EventKind::Lap, car, car, track.pos[sim.track_idx[car] as usize], V2::ZERO, race.lap[car] as f32, src));
        } else if prev < len * 0.25 && s_new > len * 0.75 {
            race.lap[car] = race.lap[car].saturating_sub(1);
        }
        race.s[car] = s_new;
        race.total[car] = race.lap[car] as f32 * len + s_new;
    }
    let mut drifting = false;
    for e in &ev.e {
        match e.kind {
            EventKind::Drift if e.car == 0 => {
                drifting = true;
                race.drift_score += e.mag * dt * 10.0 * (1.0 + race.combo * 0.25);
            }
            EventKind::NearMiss if e.car == 0 => {
                race.drift_score += 250.0;
                race.near_misses += 1;
                race.combo += 1.0;
            }
            EventKind::Crash if e.car == 0 => race.combo = 0.0,
            _ => {}
        }
    }
    if drifting {
        race.combo_timer = 1.0;
        race.combo = (race.combo + dt * 0.5).min(9.0);
    } else {
        race.combo_timer -= dt;
        if race.combo_timer <= 0.0 {
            race.combo = 0.0;
        }
    }
    ev.e.extend(new_events);
}

fn race_positions(ctx: &mut SysCtx) {
    let src = ctx.id();
    let mut race = ctx.write::<RaceState>();
    let mut ev = ctx.write::<Events>();
    let mut order: Vec<usize> = (0..CARS).collect();
    order.sort_by(|&a, &b| race.total[b].total_cmp(&race.total[a]));
    for (p, &car) in order.iter().enumerate() {
        race.position[car] = p as u8 + 1;
    }
    let pp = race.position[0];
    if pp < race.prev_player_pos {
        ev.e.push(event(EventKind::Overtake, 0, 0, V2::ZERO, V2::ZERO, pp as f32, src));
    }
    race.prev_player_pos = pp;
}

// ------------------------------------------------------------------ direção

fn director(ctx: &mut SysCtx) {
    let dt = ctx.dt();
    let tick = ctx.tick();
    let ev = ctx.read::<Events>();
    let hot = ctx.read::<VehicleHot>();
    let sim = ctx.read::<VehicleSim>();
    let ctl = ctx.read::<VehicleControl>();
    let race = ctx.read::<RaceState>();
    let mut rigw = ctx.write::<CameraRig>();
    let mut tc = ctx.write::<TimeControl>();
    let mut vfxw = ctx.write::<VfxRequests>();
    let rig = &mut *rigw;
    let vfx = &mut vfxw.r;
    let p0 = hot.pos[0];
    let v0 = hot.vel[0];
    let speed = v0.len();
    let req = |kind: VfxKind, pos: V2, dir: V2, mag: f32, color: u32| VfxReq { kind, pos, dir, mag, color };
    let banner = |rig: &mut CameraRig, b: u8, val: f32| {
        rig.banner = b;
        rig.banner_t = 1.6;
        rig.banner_val = val;
    };

    rig.near_cd -= dt;
    rig.ink_cd -= dt;
    rig.banner_t -= dt;

    for e in &ev.e {
        let car = e.car as usize;
        let near_player = e.car == 0 || e.other == 0 || (e.pos - p0).len() < 45.0;
        match e.kind {
            EventKind::Crash => {
                vfx.push(req(VfxKind::Debris, e.pos, e.dir, e.mag, CAR_COLORS[car]));
                vfx.push(req(VfxKind::Sparks, e.pos, e.dir, e.mag, 0));
                vfx.push(req(VfxKind::Smoke, e.pos, e.dir, e.mag, 0));
                rig.shake += if near_player { 1.2 } else { 0.3 };
                if near_player && tc.hold <= 0.0 {
                    // o crash cinematográfico: câmera lenta, foco no impacto, ângulo holandês
                    tc.target = 0.22;
                    tc.hold = 0.55;
                    rig.cine_target = 1.0;
                    rig.focus = e.pos;
                    rig.flash = 1.0;
                    rig.seed = tick as u32;
                    vfx.push(req(VfxKind::Ring, e.pos, e.dir, 1.5, 0));
                    vfx.push(req(VfxKind::Ink, e.pos, e.dir, 1.0, 0));
                    banner(rig, BANNER_CRASH, e.mag * 3.6);
                }
            }
            EventKind::Impact | EventKind::Wall => {
                vfx.push(req(VfxKind::Sparks, e.pos, e.dir, e.mag, 0));
                if near_player {
                    rig.shake += (e.mag * 0.03).min(0.6);
                }
                if e.car == 0 && e.kind == EventKind::Impact && rig.ink_cd <= 0.0 {
                    vfx.push(req(VfxKind::Ink, e.pos, e.dir, 0.7, 0));
                    rig.ink_cd = 0.3;
                }
            }
            EventKind::NearMiss => {
                if rig.near_cd <= 0.0 {
                    rig.near_cd = 1.2;
                    tc.target = tc.target.min(0.5);
                    tc.hold = tc.hold.max(0.15);
                    vfx.push(req(VfxKind::Ring, e.pos, e.dir, 1.0, 0));
                    vfx.push(req(VfxKind::Ink, e.pos, e.dir, 1.0, 0));
                    rig.speedlines = 1.0;
                    banner(rig, BANNER_NEAR, 250.0);
                }
            }
            EventKind::Drift if e.car == 0 => {
                if rig.ink_cd <= 0.0 {
                    rig.ink_cd = 0.07;
                    vfx.push(req(VfxKind::Ink, e.pos, e.dir, 0.45, 0));
                }
                if e.mag > 22.0 && rig.banner_t <= 0.0 {
                    banner(rig, BANNER_DRIFT, e.mag);
                }
            }
            EventKind::Lap if e.car == 0 => {
                vfx.push(req(VfxKind::Confetti, p0, V2::ZERO, 1.0, 0));
                if race.finished[0] {
                    banner(rig, BANNER_FINISH, race.position[0] as f32);
                } else if race.lap[0] >= 2 {
                    banner(rig, BANNER_LAP, race.lap[0] as f32);
                }
            }
            EventKind::Overtake if e.car == 0 => {
                vfx.push(req(VfxKind::Confetti, p0, V2::ZERO, 0.5, 0));
                banner(rig, BANNER_OVERTAKE, e.mag);
            }
            _ => {}
        }
    }
    if tc.hold > 0.0 {
        tc.hold -= dt;
        if tc.hold <= 0.0 {
            tc.target = 1.0;
            rig.cine_target = 0.0;
        }
    }

    // câmera
    let base_zoom = lerp(12.0, 8.5, (speed / 85.0).clamp(0.0, 1.0));
    let follow = p0 + v0 * 0.35;
    rig.cine += (rig.cine_target - rig.cine) * (dt * 6.0).min(1.0);
    let target = follow.lerp(rig.focus, rig.cine * 0.75);
    rig.pos += (target - rig.pos) * (dt * 8.0).min(1.0);
    let target_zoom = lerp(base_zoom, 19.0, rig.cine);
    rig.zoom += (target_zoom - rig.zoom) * (dt * 4.0).min(1.0);
    let dutch = if rig.seed & 1 == 0 { 0.14 } else { -0.14 };
    rig.angle = dutch * rig.cine;
    rig.shake = rig.shake.min(2.0);
    let mut rng = Rng::new(tick.wrapping_mul(31) + 7);
    rig.shake_off = v2(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)) * (rig.shake * 0.7);
    rig.shake *= (1.0 - 5.0 * dt).max(0.0);
    rig.flash *= (1.0 - 4.0 * dt).max(0.0);
    let boosting = ctl.c[0].boost && sim.boost[0] > 0.0;
    let sl = ((speed - 50.0) / 30.0).clamp(0.0, 1.0) * 0.6 + if boosting { 0.5 } else { 0.0 };
    rig.speedlines += (sl - rig.speedlines) * (dt * 5.0).min(1.0);
}

// ------------------------------------------------------------------ VFX

const INK: [u32; 4] = [0xFF2FA0, 0x22E0FF, 0xFFE600, 0x9DFF3A];

fn vfx_simulate(ctx: &mut SysCtx) {
    let dt = ctx.dt();
    let tick = ctx.tick();
    let faults = ctx.read::<DebugFaults>().bits;
    let req = ctx.read::<VfxRequests>();
    let hot = ctx.read::<VehicleHot>();
    let sim = ctx.read::<VehicleSim>();
    let mut pw = ctx.write::<Particles>();
    let p = &mut *pw;
    if faults & F_PANIC_VFX != 0 {
        // F6: bug clássico de índice. A política Disable desfaz e desliga o VFX; o jogo segue.
        let bad = p.pos.len() + 5;
        p.pos[bad] = V2::ZERO;
    }
    let mut rng = Rng::new(tick ^ 0x5EED_F00D);
    let rand_dir = |rng: &mut Rng| V2::from_angle(rng.range(0.0, std::f32::consts::TAU));

    for r in &req.r {
        match r.kind {
            VfxKind::Sparks => {
                let n = (r.mag * 1.2).clamp(6.0, 50.0) as usize;
                for _ in 0..n {
                    let base = if r.dir.len2() > 0.0 { r.dir } else { rand_dir(&mut rng) };
                    let d = base.rot(rng.range(-1.0, 1.0));
                    let sp = rng.range(8.0, 28.0) * (0.5 + r.mag / 40.0);
                    let col = rng.pick(&[0xFFE58A, 0xFFB347, 0xFFFFFF]);
                    p.spawn(r.pos, d * sp, rng.range(0.2, 0.55), rng.range(0.12, 0.22), col, P_SPARK, 0.0);
                }
            }
            VfxKind::Debris => {
                for _ in 0..36 {
                    let v = rand_dir(&mut rng) * rng.range(4.0, 10.0 + r.mag * 0.6) + r.dir * (r.mag * 0.2);
                    let col = forja::draw::scale(r.color, rng.range(0.6, 1.1));
                    p.spawn(r.pos, v, rng.range(1.4, 2.6), rng.range(0.25, 0.6), col, P_DEBRIS, rng.range(0.0, std::f32::consts::TAU));
                }
            }
            VfxKind::Smoke => {
                for _ in 0..22 {
                    let off = rand_dir(&mut rng) * rng.range(0.0, 1.5);
                    let g = rng.range(70.0, 130.0) as u8;
                    p.spawn(r.pos + off, rand_dir(&mut rng) * rng.range(1.0, 5.0), rng.range(1.2, 2.8), rng.range(1.0, 2.2), forja::draw::rgb(g, g, g), P_SMOKE, 0.0);
                }
            }
            VfxKind::Ink => {
                let n = (8.0 * r.mag).clamp(3.0, 18.0) as usize;
                for _ in 0..n {
                    let off = rand_dir(&mut rng) * rng.range(0.5, 3.5) * r.mag;
                    let size = rng.range(0.5, 1.2) * r.mag.clamp(0.6, 1.6);
                    p.spawn(r.pos + off, rand_dir(&mut rng) * rng.range(0.5, 3.0), rng.range(1.0, 1.9), size, rng.pick(&INK), P_INK, rng.range(0.0, std::f32::consts::TAU));
                }
            }
            VfxKind::Ring => p.spawn(r.pos, v2(r.mag * 18.0, 0.0), 0.45, 1.0, 0xFFFFFF, P_RING, 0.0),
            VfxKind::Confetti => {
                let n = (30.0 * r.mag) as usize;
                for _ in 0..n {
                    p.spawn(r.pos, rand_dir(&mut rng) * rng.range(4.0, 14.0), rng.range(1.2, 2.0), rng.range(0.25, 0.4), rng.pick(&INK), P_CONFETTI, rng.range(0.0, std::f32::consts::TAU));
                }
            }
        }
    }
    // emissão contínua: fumaça de drift e poeira fora da pista
    for car in 0..CARS {
        let speed = hot.vel[car].len();
        let f = V2::from_angle(hot.angle[car]);
        let r = f.perp();
        if sim.drift[car] > 0.25 && speed > 12.0 {
            for side in [-0.85f32, 0.85] {
                let pos = hot.pos[car] - f * 1.6 + r * side;
                p.spawn(pos, hot.vel[car] * -0.05 + rand_dir(&mut rng) * 0.8, rng.range(0.8, 1.6), rng.range(0.5, 0.9), 0xD8D8D8, P_SMOKE, 0.0);
            }
        }
        if sim.offroad[car] && speed > 10.0 {
            p.spawn(hot.pos[car] - f * 1.8, hot.vel[car] * -0.1 + rand_dir(&mut rng), rng.range(0.6, 1.2), rng.range(0.6, 1.0), 0xC2A878, P_DUST, 0.0);
        }
    }
    // integração e remoção (swap_remove mantém o pool denso)
    let mut i = 0;
    while i < p.pos.len() {
        p.life[i] -= dt;
        if p.life[i] <= 0.0 {
            p.swap_remove(i);
            continue;
        }
        let k = p.kind[i];
        let drag = match k {
            P_SPARK => 3.0,
            P_SMOKE => 1.5,
            P_DUST => 2.0,
            P_DEBRIS => 2.2,
            P_INK => 4.0,
            P_CONFETTI => 2.5,
            _ => 0.0,
        };
        let v = p.vel[i];
        if k == P_RING {
            p.size[i] += v.x * dt;
        } else {
            p.pos[i] += v * dt;
            p.vel[i] = v * (1.0 - drag * dt).max(0.0);
        }
        if k == P_SMOKE || k == P_DUST {
            p.size[i] += dt * 0.9;
        }
        if k == P_DEBRIS || k == P_CONFETTI {
            p.rot[i] += dt * 6.0 * v.len().min(10.0) / 10.0;
        }
        i += 1;
    }
}

// ------------------------------------------------------------------ fronteira com o render

fn render_extract(ctx: &mut SysCtx) {
    let hot = ctx.read::<VehicleHot>();
    let sim = ctx.read::<VehicleSim>();
    let ctl = ctx.read::<VehicleControl>();
    let crash = ctx.read::<CrashState>();
    let obs = ctx.read::<Obstacles>();
    let parts = ctx.read::<Particles>();
    let rig = ctx.read::<CameraRig>();
    let race = ctx.read::<RaceState>();
    let mut snapw = ctx.write::<RenderSnapshot>();
    let s = &mut *snapw;

    std::mem::swap(&mut s.cars, &mut s.cars_prev);
    s.cars.clear();
    for car in 0..CARS {
        s.cars.push(CarSnap {
            pos: hot.pos[car],
            angle: hot.angle[car],
            color: CAR_COLORS[car],
            brake: ctl.c[car].brake,
            crashing: crash.c[car].mode == MODE_CRASHING,
            boost: ctl.c[car].boost && sim.boost[car] > 0.0,
            ghost: crash.c[car].invuln > 0.0,
        });
    }
    if !s.valid {
        s.cars_prev = s.cars.clone();
    }
    s.obstacles.clear();
    s.obstacles.extend(obs.o.iter().map(|o| ObsSnap { pos: o.pos, radius: o.radius, kind: o.kind, color: o.color, angle: o.angle }));
    s.parts.clear();
    for i in 0..parts.pos.len() {
        s.parts.push(PartSnap {
            pos: parts.pos[i],
            vel: parts.vel[i],
            t: parts.life[i] / parts.max_life[i],
            size: parts.size[i],
            color: parts.color[i],
            kind: parts.kind[i],
            rot: parts.rot[i],
        });
    }
    let cam = CamSnap { pos: rig.pos + rig.shake_off, zoom: rig.zoom, angle: rig.angle, cine: rig.cine, flash: rig.flash, speedlines: rig.speedlines };
    s.cam_prev = if s.valid { s.cam } else { cam };
    s.cam = cam;
    s.hud = Hud {
        speed: hot.vel[0].len() * 3.6,
        lap: race.lap[0].clamp(1, LAPS),
        position: race.position[0],
        drift_score: race.drift_score,
        combo: race.combo,
        race_time: race.race_time,
        best: race.best[0],
        banner: if rig.banner_t > 0.0 { rig.banner } else { BANNER_NONE },
        banner_t: rig.banner_t,
        banner_val: rig.banner_val,
        boost: sim.boost[0],
        finished: race.finished[0],
    };
    s.valid = true;
}

fn determinism_hash(ctx: &mut SysCtx) {
    let tick = ctx.tick();
    let hot = ctx.read::<VehicleHot>();
    let mut src = ctx.write::<InputSource>();
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    let mut mixf = |x: f32| {
        h ^= x.to_bits() as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    };
    for i in 0..hot.pos.len() {
        mixf(hot.pos[i].x);
        mixf(hot.pos[i].y);
        mixf(hot.vel[i].x);
        mixf(hot.vel[i].y);
        mixf(hot.angle[i]);
    }
    let expected = src.replay.as_ref().and_then(|l| l.hashes.iter().find(|(t, _)| *t == tick).map(|(_, e)| *e));
    if let Some(exp) = expected {
        src.checked += 1;
        if exp != h && src.divergence.is_none() {
            src.divergence = Some(tick);
            forja::crash::breadcrumb(format!("DIVERGÊNCIA de determinismo no tick {}: o replay não reproduz o estado gravado", tick));
        }
    }
    forja::crash::record_hash(tick, h);
}
