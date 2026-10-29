//! Editor em overlay, sobre o jogo rodando. Quatro visões do mesmo "banco":
//! F1 timeline do frame (com caminho crítico), F2 matriz sistemas x tabelas,
//! F3 tabelas (layout, bytes, uso de cache), F4 falhas e relatórios.

use crate::analysis::{self, CritPath};
use crate::crash;
use crate::draw::{rgb, Canvas};
use crate::platform::{vk, Keys, Text, CHAR_W, LINE_H};
use crate::profiler::{self, FrameRec, NO_SUB, NO_SYS, RENDER_TICK};
use crate::schedule::{Schedule, SUB_INV, SUB_TXN};
use crate::world::World;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Off,
    Timeline,
    Matrix,
    Tables,
    Failures,
}

pub struct Editor {
    pub view: View,
    pub frozen: Option<FrameRec>,
    pub worst: bool,
    pub selected: usize,
    pub invariants: bool,
    /// Linhas extras fornecidas pelo jogo (estatísticas de eventos, replay...).
    pub extra: Vec<String>,
}

const BG: u32 = rgb(14, 16, 22);
const FG: u32 = 0xE5E7EB;
const DIM: u32 = 0x9CA3AF;
const ACCENT: u32 = 0xFFE14D;
const BAD: u32 = 0xF87171;
const GOOD: u32 = 0x4ADE80;
const BUDGET_MS: f64 = 1000.0 / 60.0;

fn fit(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else if n <= 1 {
        String::new()
    } else {
        let mut o: String = s.chars().take(n - 1).collect();
        o.push('…');
        o
    }
}

fn txt(t: &mut Vec<Text>, x: i32, y: i32, color: u32, s: impl Into<String>) {
    t.push(Text::new(x, y, color, s));
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new()
    }
}

impl Editor {
    pub fn new() -> Editor {
        Editor { view: View::Off, frozen: None, worst: false, selected: 0, invariants: true, extra: Vec::new() }
    }

    /// Retorna true se o usuário pediu para religar sistemas desligados.
    pub fn handle_keys(&mut self, k: &Keys, n_systems: usize) -> bool {
        let toggle = |cur: View, v: View| if cur == v { View::Off } else { v };
        if k.pressed(vk::F1) {
            self.view = toggle(self.view, View::Timeline);
        }
        if k.pressed(vk::F2) {
            self.view = toggle(self.view, View::Matrix);
        }
        if k.pressed(vk::F3) {
            self.view = toggle(self.view, View::Tables);
        }
        if k.pressed(vk::F4) {
            self.view = toggle(self.view, View::Failures);
        }
        if k.pressed(b'P') {
            self.frozen = match self.frozen {
                Some(_) => None,
                None => self.pick_frame(),
            };
        }
        if k.pressed(b'W') {
            self.worst = !self.worst;
            if self.frozen.is_some() {
                self.frozen = None;
                self.frozen = self.pick_frame();
            }
        }
        if k.pressed(b'I') {
            self.invariants = !self.invariants;
            crash::breadcrumb(format!("invariantes {}", if self.invariants { "ligadas" } else { "desligadas" }));
        }
        if n_systems > 0 {
            if k.pressed(vk::PGDN) {
                self.selected = (self.selected + 1) % n_systems;
            }
            if k.pressed(vk::PGUP) {
                self.selected = (self.selected + n_systems - 1) % n_systems;
            }
        }
        k.pressed(b'R')
    }

    fn pick_frame(&self) -> Option<FrameRec> {
        if let Some(f) = &self.frozen {
            return Some(f.clone());
        }
        profiler::get().with_frames(|ring| {
            if self.worst {
                // pior frame dos últimos ~5 s, ignorando o aquecimento
                ring.iter().rev().take(300).max_by_key(|f| f.dur()).cloned()
            } else {
                ring.back().cloned()
            }
        })
    }

    pub fn draw(&mut self, c: &mut Canvas, t: &mut Vec<Text>, world: &World, scheds: &[&Schedule]) {
        let help = "F1 timeline  F2 matriz  F3 tabelas  F4 falhas | P congelar  W pior frame  I invariantes  R religar  PgUp/PgDn sistema";
        c.fill_rect(0, 0, c.w as i32, 18, BG, 0.75);
        txt(t, 6, 2, DIM, help);
        let frame = self.pick_frame();
        match self.view {
            View::Off => {}
            View::Timeline => self.draw_timeline(c, t, scheds, frame.as_ref()),
            View::Matrix => self.draw_matrix(c, t, world, scheds, frame.as_ref()),
            View::Tables => self.draw_tables(c, t, world, scheds),
            View::Failures => self.draw_failures(c, t, scheds),
        }
    }

    // ------------------------------------------------------------ helpers

    fn locate<'s>(scheds: &[&'s Schedule], id: u16) -> Option<(&'s Schedule, usize)> {
        for s in scheds {
            let i = id.wrapping_sub(s.base_id) as usize;
            if id >= s.base_id && i < s.systems.len() {
                return Some((s, i));
            }
        }
        None
    }

    /// Duração (ms) de cada sistema no frame, para o caminho crítico daquele frame.
    /// Grafo de simulação usa o primeiro tick do frame; sem ticks, usa a média.
    fn frame_durs(f: Option<&FrameRec>, s: &Schedule) -> Vec<f64> {
        let mut d = vec![0.0; s.systems.len()];
        let mut found = false;
        if let Some(f) = f {
            for z in &f.zones {
                if z.sub != NO_SUB || z.sys == NO_SYS {
                    continue;
                }
                let want = if s.is_render { RENDER_TICK } else { 0 };
                if z.tick != want || z.sys < s.base_id {
                    continue;
                }
                let i = (z.sys - s.base_id) as usize;
                if i < d.len() {
                    d[i] += z.dur() as f64 / 1e6;
                    found = true;
                }
            }
        }
        if found {
            d
        } else {
            analysis::avg_durations(s)
        }
    }

    // ------------------------------------------------------------ F1 timeline

    fn draw_timeline(&self, c: &mut Canvas, t: &mut Vec<Text>, scheds: &[&Schedule], frame: Option<&FrameRec>) {
        let Some(f) = frame else { return };
        let lanes = f.zones.iter().map(|z| z.lane as usize + 1).max().unwrap_or(1).max(2);
        let lane_h = 18;
        let graph_h = 56;
        let panel_h = 30 + graph_h + 26 + lanes as i32 * lane_h + 24;
        let top = c.h as i32 - panel_h;
        let w = c.w as i32;
        c.fill_rect(0, top, w, c.h as i32, BG, 0.88);

        // histórico de frames
        let gx = 10;
        let gy = top + 26;
        let (hist, worst_idx) = profiler::get().with_frames(|ring| {
            let v: Vec<(u64, f64, u8)> = ring.iter().rev().take(240).map(|fr| (fr.index, fr.dur() as f64 / 1e6, fr.ticks)).collect();
            (v, ring.iter().rev().take(300).max_by_key(|fr| fr.dur()).map(|fr| fr.index))
        });
        let max_ms = 40.0;
        c.fill_rect(gx, gy, gx + 480, gy + graph_h, rgb(28, 31, 40), 1.0);
        for (k, (idx, ms, _)) in hist.iter().enumerate() {
            let x = gx + 480 - (k as i32 + 1) * 2;
            let hpx = ((ms / max_ms).min(1.0) * graph_h as f64) as i32;
            let col = if *idx == f.index {
                0xFFFFFF
            } else if *ms > BUDGET_MS * 2.0 {
                BAD
            } else if *ms > BUDGET_MS {
                ACCENT
            } else {
                GOOD
            };
            c.fill_rect(x, gy + graph_h - hpx, x + 2, gy + graph_h, col, 0.9);
        }
        let by = gy + graph_h - ((BUDGET_MS / max_ms) * graph_h as f64) as i32;
        c.fill_rect(gx, by, gx + 480, by + 1, BAD, 0.8);
        txt(t, gx + 484, by - 7, BAD, "16.7 ms");

        // caminho crítico deste frame
        let cps: Vec<(CritPath, &Schedule)> = scheds.iter().map(|s| (analysis::critical_path(s, &Self::frame_durs(Some(f), s)), *s)).collect();
        let frame_ms = f.dur() as f64 / 1e6;
        let mode = if self.frozen.is_some() { "CONGELADO" } else if self.worst { "pior frame (5 s)" } else { "ao vivo" };
        txt(
            t,
            10,
            top + 6,
            FG,
            format!(
                "Frame #{}  {:.2} ms  ticks de simulação: {}  timescale {:.2}   [{}]{}",
                f.index,
                frame_ms,
                f.ticks,
                f.timescale,
                mode,
                if Some(f.index) == worst_idx { "  ← pior dos últimos 5 s" } else { "" }
            ),
        );
        let mut ty = gy;
        for (cp, s) in &cps {
            let names: Vec<&str> = cp.path.iter().map(|&i| s.systems[i].decl.name).collect();
            txt(t, gx + 560, ty, ACCENT, format!("caminho crítico [{}] {:.2} ms   paralelismo {:.2}x", s.name, cp.span, cp.parallelism()));
            txt(t, gx + 560, ty + LINE_H, DIM, fit(&names.join(" → "), ((w - gx - 570) / CHAR_W) as usize));
            ty += LINE_H * 2 + 2;
        }

        // lanes
        let lx = 76;
        let ly = gy + graph_h + 22;
        let tw = (w - lx - 12) as f64;
        let span_ms = frame_ms.max(BUDGET_MS) * 1.04;
        let to_x = |ns: u64| lx + ((ns.saturating_sub(f.start) as f64 / 1e6) / span_ms * tw) as i32;
        for l in 0..lanes {
            let y = ly + l as i32 * lane_h;
            c.fill_rect(lx, y, w - 12, y + lane_h - 2, if l % 2 == 0 { rgb(24, 27, 35) } else { rgb(20, 23, 30) }, 1.0);
            txt(t, 8, y + 1, DIM, crash::lane_name(l));
        }
        let mut ms = 0.0;
        while ms < span_ms {
            let x = lx + (ms / span_ms * tw) as i32;
            c.fill_rect(x, ly - 4, x + 1, ly + lanes as i32 * lane_h, 0x3A3F4B, 0.6);
            if (ms as i32) % 2 == 0 {
                txt(t, x + 2, ly - 18, 0x6B7280, format!("{}ms", ms as i32));
            }
            ms += 1.0;
        }
        let bx = lx + (BUDGET_MS / span_ms * tw) as i32;
        c.fill_rect(bx, ly - 4, bx + 2, ly + lanes as i32 * lane_h, BAD, 0.9);

        let mut labels: Vec<(i32, i32, i32, String)> = Vec::new();
        for pass in 0..2 {
            for z in &f.zones {
                let is_parent = z.sub == NO_SUB;
                if (pass == 0) != is_parent {
                    continue;
                }
                let y = ly + z.lane as i32 * lane_h;
                let x0 = to_x(z.start);
                let x1 = (to_x(z.end)).max(x0 + 1);
                let info = Self::locate(scheds, z.sys);
                let color = info.map(|(s, i)| s.systems[i].decl.phase.color()).unwrap_or(0x9CA3AF);
                match z.sub {
                    NO_SUB => {
                        c.fill_rect(x0, y + 2, x1, y + lane_h - 4, crate::draw::scale(color, 0.75), 0.95);
                        let critical = info
                            .map(|(s, i)| {
                                let want = if s.is_render { RENDER_TICK } else { 0 };
                                z.tick == want && cps.iter().any(|(cp, cs)| std::ptr::eq(*cs, s) && cp.on_path(i))
                            })
                            .unwrap_or(false);
                        if critical {
                            c.rect(x0, y + 1, x1, y + lane_h - 3, ACCENT, 1.0);
                        }
                        if x1 - x0 > 5 * CHAR_W {
                            labels.push((x0 + 3, y + 1, x1 - x0, z.name.to_string()));
                        }
                    }
                    SUB_TXN => c.fill_rect(x0, y + lane_h - 7, x1, y + lane_h - 4, 0xE11D48, 1.0),
                    SUB_INV => c.fill_rect(x0, y + lane_h - 7, x1, y + lane_h - 4, 0x22D3EE, 1.0),
                    _ => {
                        c.fill_rect(x0, y + 5, x1, y + lane_h - 7, color, 0.95);
                    }
                }
            }
        }
        for (x, y, wpx, s) in labels {
            txt(t, x, y, 0x0B0D12, fit(&s, (wpx / CHAR_W) as usize));
        }
        // legenda
        let lgy = ly + lanes as i32 * lane_h + 4;
        let mut x = lx;
        for ph in [
            crate::schedule::Phase::Input,
            crate::schedule::Phase::AI,
            crate::schedule::Phase::Control,
            crate::schedule::Phase::Physics,
            crate::schedule::Phase::Collision,
            crate::schedule::Phase::Gameplay,
            crate::schedule::Phase::Direction,
            crate::schedule::Phase::VFX,
            crate::schedule::Phase::Extract,
            crate::schedule::Phase::Render,
        ] {
            c.fill_rect(x, lgy + 3, x + 10, lgy + 13, ph.color(), 1.0);
            txt(t, x + 14, lgy, DIM, ph.name());
            x += 24 + ph.name().len() as i32 * CHAR_W;
        }
        c.fill_rect(x, lgy + 9, x + 12, lgy + 12, 0xE11D48, 1.0);
        txt(t, x + 16, lgy, DIM, "transação");
        x += 100;
        c.fill_rect(x, lgy + 9, x + 12, lgy + 12, 0x22D3EE, 1.0);
        txt(t, x + 16, lgy, DIM, "invariantes");
        x += 110;
        c.rect(x, lgy + 2, x + 14, lgy + 13, ACCENT, 1.0);
        txt(t, x + 18, lgy, DIM, "caminho crítico");
    }

    // ------------------------------------------------------------ F2 matriz

    fn draw_matrix(&self, c: &mut Canvas, t: &mut Vec<Text>, world: &World, scheds: &[&Schedule], frame: Option<&FrameRec>) {
        let w = c.w as i32;
        let h = c.h as i32;
        c.fill_rect(0, 18, w, h, BG, 0.9);
        let used: Vec<usize> = (0..world.slots.len()).filter(|&tb| scheds.iter().any(|s| s.systems.iter().any(|x| x.reads.contains(&tb)))).collect();
        let x_name = 22;
        let x_info = 196;
        let mx = 430;
        let cw = ((w - mx - 10) / used.len().max(1) as i32).min(46);
        let mut y = 26;
        txt(t, 10, y, FG, "Matriz de acesso: R lê, W escreve, * só algumas colunas.  ● = caminho crítico deste frame.");
        y += 20;
        for (k, &tb) in used.iter().enumerate() {
            let name = fit(world.slots[tb].decl.name, ((2 * cw) / CHAR_W - 1).max(3) as usize);
            let cx = mx + k as i32 * cw + cw / 2;
            let yy = y + (k as i32 % 2) * LINE_H;
            txt(t, (cx - name.chars().count() as i32 * CHAR_W / 2).max(mx - 40), yy, DIM, name);
            c.fill_rect(cx, yy + LINE_H - 1, cx + 1, y + 2 * LINE_H + 4, 0x4B5563, 0.8);
        }
        y += 2 * LINE_H + 6;
        txt(t, x_info, y - LINE_H - 2, DIM, "freq  falha     ms   folga");

        let row_h = 17;
        let mut flat = 0usize;
        let mut sel: Option<(&Schedule, usize)> = None;
        for s in scheds {
            let durs = Self::frame_durs(frame, s);
            let cp = analysis::critical_path(s, &durs);
            c.fill_rect(8, y, w - 8, y + row_h, rgb(34, 38, 50), 1.0);
            txt(
                t,
                12,
                y + 1,
                ACCENT,
                format!("grafo '{}'   caminho crítico {:.2} ms   trabalho {:.2} ms   paralelismo {:.2}x", s.name, cp.span, cp.work, cp.parallelism()),
            );
            y += row_h + 2;
            for (i, sys) in s.systems.iter().enumerate() {
                let selected = flat == self.selected;
                if selected {
                    c.fill_rect(8, y, w - 8, y + row_h, rgb(55, 48, 20), 1.0);
                    sel = Some((s, i));
                }
                flat += 1;
                let enabled = sys.enabled.load(std::sync::atomic::Ordering::Relaxed);
                c.fill_rect(10, y + 3, 18, y + row_h - 3, sys.decl.phase.color(), 1.0);
                if cp.on_path(i) {
                    txt(t, x_name - 2, y + 1, ACCENT, "●");
                }
                let name = if enabled { sys.decl.name.to_string() } else { format!("{} [OFF]", sys.decl.name) };
                txt(t, x_name + 12, y + 1, if enabled { FG } else { BAD }, fit(&name, 22));
                let freq = if s.is_render { "frame".to_string() } else { format!("{}Hz", 60 / sys.decl.every) };
                txt(t, x_info, y + 1, DIM, format!("{:<5} {:<5} {:6.3} {:6.3}", freq, sys.decl.policy.short(), durs[i], cp.slack[i]));
                for (k, &tb) in used.iter().enumerate() {
                    let x0 = mx + k as i32 * cw + 2;
                    if let Some(a) = sys.access_to(tb) {
                        let (col, label) = if a.write { (0xF97316, "W") } else { (0x3B82F6, "R") };
                        c.fill_rect(x0, y + 1, x0 + cw - 4, y + row_h - 1, col, if a.write { 0.9 } else { 0.7 });
                        let label = if a.cols.is_empty() { label.to_string() } else { format!("{}*", label) };
                        txt(t, x0 + cw / 2 - 6, y + 1, 0xFFFFFF, label);
                    }
                }
                y += row_h;
            }
            y += 6;
        }

        // detalhes do sistema selecionado
        if let Some((s, i)) = sel {
            let sys = &s.systems[i];
            let mut lines = vec![format!(
                "{}  [{}]  política: {}  {}",
                sys.decl.name,
                sys.decl.phase.name(),
                sys.decl.policy.name(),
                if sys.decl.note.is_empty() { String::new() } else { format!("- {}", sys.decl.note) }
            )];
            let why = |a: &crate::schedule::SysRt, b: &crate::schedule::SysRt| -> String {
                a.access
                    .iter()
                    .filter(|x| b.access.iter().any(|y| y.table == x.table && (x.write || y.write)))
                    .map(|x| world.slots[x.table].decl.name)
                    .collect::<Vec<_>>()
                    .join("/")
            };
            let deps: Vec<String> = sys.deps.iter().map(|&d| format!("{} ({})", s.systems[d].decl.name, why(sys, &s.systems[d]))).collect();
            let outs: Vec<String> = sys.dependents.iter().map(|&d| s.systems[d].decl.name.to_string()).collect();
            lines.push(format!("espera por: {}", if deps.is_empty() { "nada (raiz do grafo)".into() } else { deps.join(", ") }));
            lines.push(format!("libera: {}", if outs.is_empty() { "nada (folha)".into() } else { outs.join(", ") }));
            for a in &sys.access {
                if let Some(st) = analysis::access_stat(world, a.table, &a.cols) {
                    lines.push(format!(
                        "  {} {}: {} úteis de {} carregados ({:.0}%)",
                        if a.write { "W" } else { "R" },
                        world.slots[a.table].decl.name,
                        analysis::fmt_bytes(st.useful),
                        analysis::fmt_bytes(st.loaded),
                        st.util() * 100.0
                    ));
                }
            }
            for l in lines {
                txt(t, 12, y, if y < h - 4 * LINE_H { FG } else { DIM }, fit(&l, ((w - 24) / CHAR_W) as usize));
                y += LINE_H;
            }
        }
        y += 6;
        let durs: Vec<Vec<f64>> = scheds.iter().map(|s| Self::frame_durs(frame, s)).collect();
        txt(t, 12, y, ACCENT, "Sugestões:");
        y += LINE_H;
        for ins in analysis::insights(world, scheds, &durs) {
            if y > h - LINE_H {
                break;
            }
            txt(t, 18, y, DIM, fit(&format!("• {}", ins), ((w - 30) / CHAR_W) as usize));
            y += LINE_H;
        }
    }

    // ------------------------------------------------------------ F3 tabelas

    fn draw_tables(&self, c: &mut Canvas, t: &mut Vec<Text>, world: &World, scheds: &[&Schedule]) {
        let w = c.w as i32;
        c.fill_rect(0, 18, w, c.h as i32, BG, 0.9);
        let mut y = 26;
        txt(t, 10, y, FG, "Tabelas: o banco de dados do jogo. 'uso' = bytes úteis / bytes trazidos ao cache no pior acesso declarado.");
        y += 22;
        txt(
            t,
            10,
            y,
            DIM,
            format!("{:<16} {:<11} {:<4} {:>7} {:>6} {:>4} {:>9} {:>2} {:>2} {:>3} {:>5}  {}", "tabela", "vida", "lay", "linhas", "B/lin", "pad", "total", "W", "R", "inv", "uso", "nota"),
        );
        y += LINE_H + 4;
        for (tb, slot) in world.slots.iter().enumerate() {
            let d = &slot.decl;
            let writers = scheds.iter().flat_map(|s| s.systems.iter()).filter(|x| x.writes.contains(&tb)).count();
            let readers = scheds.iter().flat_map(|s| s.systems.iter()).filter(|x| x.reads.contains(&tb) && !x.writes.contains(&tb)).count();
            let pad = if d.layout == crate::world::Layout::AoS { d.row_size.saturating_sub(d.used_bytes()) } else { 0 };
            let worst = scheds
                .iter()
                .flat_map(|s| s.systems.iter())
                .filter_map(|x| x.access_to(tb))
                .filter_map(|a| analysis::access_stat(world, tb, &a.cols))
                .map(|s| s.util())
                .fold(f64::NAN, f64::min);
            let uso = if worst.is_nan() { "  -".to_string() } else { format!("{:.0}%", worst * 100.0) };
            let color = if !worst.is_nan() && worst < 0.5 { BAD } else { FG };
            let inv = slot.invariant_names().count();
            let line = format!(
                "{:<16} {:<11} {:<4} {:>7} {:>6} {:>4} {:>9} {:>2} {:>2} {:>3} {:>5}  {}",
                fit(d.name, 16),
                d.lifetime.name(),
                format!("{:?}", d.layout),
                world.rows(tb),
                d.row_bytes(),
                pad,
                analysis::fmt_bytes(world.total_bytes(tb)),
                writers,
                readers,
                inv,
                uso,
                d.note
            );
            if tb % 2 == 0 {
                c.fill_rect(8, y, w - 8, y + LINE_H, rgb(24, 27, 35), 1.0);
            }
            txt(t, 10, y, color, fit(&line, ((w - 20) / CHAR_W) as usize));
            y += LINE_H + 1;
        }
        y += 10;
        let total: usize = (0..world.slots.len()).map(|tb| world.total_bytes(tb)).sum();
        let by_life = |l: crate::world::Lifetime| -> usize { (0..world.slots.len()).filter(|&tb| world.slots[tb].decl.lifetime == l).map(|tb| world.total_bytes(tb)).sum() };
        txt(
            t,
            10,
            y,
            ACCENT,
            format!(
                "Memória declarada: {}   estática {}   sessão {}   transitória {}",
                analysis::fmt_bytes(total),
                analysis::fmt_bytes(by_life(crate::world::Lifetime::Static)),
                analysis::fmt_bytes(by_life(crate::world::Lifetime::Session)),
                analysis::fmt_bytes(by_life(crate::world::Lifetime::Transient))
            ),
        );
    }

    // ------------------------------------------------------------ F4 falhas

    fn draw_failures(&self, c: &mut Canvas, t: &mut Vec<Text>, scheds: &[&Schedule]) {
        let w = c.w as i32;
        let h = c.h as i32;
        c.fill_rect(0, 18, w, h, BG, 0.9);
        let col_w = ((w / 2 - 20) / CHAR_W) as usize;
        let mut y = 26;
        txt(t, 10, y, FG, "Falhas: políticas declaradas por sistema. F5 NaN na física  F6 panic no VFX  F7 panic nas regras  F8 access violation  F9 travamento 6 s");
        y += 22;
        txt(t, 10, y, ACCENT, "Políticas de falha");
        y += LINE_H;
        for s in scheds {
            for sys in &s.systems {
                let on = sys.enabled.load(std::sync::atomic::Ordering::Relaxed);
                let fails = sys.failures.load(std::sync::atomic::Ordering::Relaxed);
                let line = format!("{:<20} {:<22} falhas: {:<3} {}", sys.decl.name, sys.decl.policy.name(), fails, if on { "" } else { "DESLIGADO (R religa)" });
                txt(t, 16, y, if on { if fails > 0 { ACCENT } else { DIM } } else { BAD }, fit(&line, col_w));
                y += LINE_H;
            }
        }
        y += 8;
        txt(t, 10, y, ACCENT, format!("Invariantes: {}", if self.invariants { "ligadas (I desliga)" } else { "DESLIGADAS (I liga)" }));
        y += LINE_H + 6;
        txt(t, 10, y, ACCENT, "Falhas nesta sessão");
        y += LINE_H;
        let f = crash::failures();
        if f.is_empty() {
            txt(t, 16, y, DIM, "nenhuma");
            y += LINE_H;
        }
        for l in f.iter().rev().take(8) {
            txt(t, 16, y, FG, fit(l, col_w));
            y += LINE_H;
        }
        y += 6;
        txt(t, 10, y, ACCENT, "Relatórios gravados");
        y += LINE_H;
        let r = crash::reports();
        if r.is_empty() {
            txt(t, 16, y, DIM, "nenhum");
        }
        for l in r.iter().rev().take(6) {
            if y > h - LINE_H {
                break;
            }
            let name = l.rsplit(['\\', '/']).next().unwrap_or(l);
            txt(t, 16, y, FG, fit(name, col_w));
            y += LINE_H;
        }

        let x2 = w / 2 + 10;
        let mut y2 = 48;
        txt(t, x2, y2, ACCENT, "Jogo");
        y2 += LINE_H;
        for l in &self.extra {
            txt(t, x2 + 6, y2, FG, fit(l, col_w));
            y2 += LINE_H;
        }
        y2 += 8;
        txt(t, x2, y2, ACCENT, "Breadcrumbs");
        y2 += LINE_H;
        let b = crash::breadcrumbs();
        let room = ((h - y2) / LINE_H).max(0) as usize;
        for l in b.iter().rev().take(room) {
            txt(t, x2 + 6, y2, DIM, fit(l, col_w));
            y2 += LINE_H;
        }
    }
}
