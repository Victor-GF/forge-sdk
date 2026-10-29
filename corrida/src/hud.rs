//! HUD do jogo: velocidade, volta, posição, placar de drift, banners estilizados,
//! minimapa e estado do replay. Roda na main thread depois do grafo de render.

use forja::draw::{rgb, Canvas};
use forja::math::v2;
use forja::platform::{Text, CHAR_W};
use forja::world::World;

use crate::tables::*;
use crate::track::Track;

#[allow(clippy::too_many_arguments)]
pub fn draw(c: &mut Canvas, t: &mut Vec<Text>, world: &World, hud: &Hud, replay: (bool, usize, Option<u64>, u32), tick: u64, show_help: bool, autopilot: bool) {
    let w = c.w as i32;
    let h = c.h as i32;

    // painel da direita
    let px = w - 250;
    c.fill_rect(px - 10, 26, w - 10, 150, rgb(10, 12, 18), 0.55);
    t.push(Text::big(px, 30, 0xFFFFFF, format!("{:>3.0} km/h", hud.speed)));
    t.push(Text::new(px, 68, 0xE5E7EB, format!("Volta {}/{}    Posição {}/{}", hud.lap, LAPS, hud.position, CARS)));
    t.push(Text::new(px, 84, 0xE5E7EB, format!("Tempo {}   Melhor {}", fmt_time(hud.race_time), if hud.best > 0.0 { fmt_time(hud.best) } else { "--".into() })));
    t.push(Text::new(px, 100, 0xFF2FA0, format!("Drift {:>8.0}   x{:.1}", hud.drift_score, 1.0 + hud.combo * 0.25)));
    t.push(Text::new(px, 118, 0x9CA3AF, "boost"));
    c.fill_rect(px + 48, 122, px + 228, 130, rgb(40, 44, 56), 1.0);
    c.fill_rect(px + 48, 122, px + 48 + (180.0 * hud.boost) as i32, 130, 0x60A5FA, 1.0);
    if hud.finished {
        t.push(Text::new(px, 134, 0x9DFF3A, "CORRIDA CONCLUÍDA"));
    } else if autopilot {
        t.push(Text::new(px, 134, 0xFACC15, "PILOTO AUTOMÁTICO (T)"));
    }

    // banner estilizado
    if hud.banner != BANNER_NONE {
        let (text, color) = match hud.banner {
            BANNER_CRASH => (format!("CRASH!  {:.0} km/h", hud.banner_val), 0xFF4D4D),
            BANNER_NEAR => (format!("NEAR MISS  +{:.0}", hud.banner_val), 0x22E0FF),
            BANNER_LAP => (format!("VOLTA {:.0}", hud.banner_val), 0xFFE600),
            BANNER_OVERTAKE => (format!("ULTRAPASSAGEM!  P{:.0}", hud.banner_val), 0x9DFF3A),
            BANNER_DRIFT => ("DRIFT!".to_string(), 0xFF2FA0),
            _ => (format!("FIM DE CORRIDA  P{:.0}", hud.banner_val), 0xFFE600),
        };
        let pop = (1.6 - hud.banner_t).clamp(0.0, 0.15) / 0.15;
        let tw = text.chars().count() as f32 * 17.0 + 60.0;
        let cx = w as f32 * 0.5;
        let y = 120.0;
        let slab = [v2(cx - tw * 0.5 * pop - 14.0, y + 44.0), v2(cx - tw * 0.5 * pop + 6.0, y), v2(cx + tw * 0.5 * pop + 14.0, y), v2(cx + tw * 0.5 * pop - 6.0, y + 44.0)];
        let shadow: Vec<_> = slab.iter().map(|p| *p + v2(6.0, 6.0)).collect();
        c.poly(&shadow, 0x000000, 0.6);
        c.poly(&slab, color, 0.92);
        if pop > 0.6 {
            t.push(Text::big((cx - text.chars().count() as f32 * 8.5) as i32, y as i32 + 6, 0x0A0A0A, text));
        }
    }

    // minimapa
    world.with::<Track, _>(|track| {
        world.with::<RenderSnapshot, _>(|snap| {
            let (x0, y0, size) = (12.0f32, 30.0f32, 170.0f32);
            c.fill_rect(x0 as i32 - 4, y0 as i32 - 4, (x0 + size) as i32 + 4, (y0 + size) as i32 + 4, rgb(10, 12, 18), 0.55);
            let sc = size / 1200.0;
            let m = |p: forja::math::V2| v2(x0 + p.x * sc, y0 + p.y * sc);
            let n = track.n();
            for i in (0..n).step_by(4) {
                let j = (i + 4) % n;
                c.line(m(track.pos[i]), m(track.pos[j]), 0xD1D5DB, 0.9);
            }
            for (i, car) in snap.cars.iter().enumerate() {
                c.circle(m(car.pos), if i == 0 { 4.0 } else { 3.0 }, car.color, 1.0);
            }
        });
    });

    // replay
    let (on, len, divergence, checked) = replay;
    if on {
        let msg = format!("REPLAY  tick {}/{}   checkpoints conferidos: {}", tick, len, checked);
        t.push(Text::new(12, 210, 0x22E0FF, msg));
    }
    if let Some(d) = divergence {
        t.push(Text::new(12, 226, 0xF87171, format!("DIVERGÊNCIA no tick {}: o estado não bate com o gravado", d)));
    }

    if show_help {
        let help = "Setas dirigir  Espaço freio de mão  Shift boost  T piloto automático  L salvar replay  Esc sair  |  F1-F4 editor  F5-F9 falhas";
        c.fill_rect(0, h - 20, w, h, rgb(10, 12, 18), 0.6);
        t.push(Text::new((w - help.chars().count() as i32 * CHAR_W) / 2, h - 18, 0xBFC5D0, help));
    }
}

fn fmt_time(s: f32) -> String {
    let m = (s / 60.0) as i32;
    format!("{}:{:04.1}", m, s - m as f32 * 60.0)
}
