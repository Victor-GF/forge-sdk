//! Desenho e interação (modo imediato): cada frame redesenha tudo a partir do projeto e
//! devolve as ações dos cliques. Três abas: Pipeline (a tabela), Grafo e Tabelas.

use forja::analysis::{access_stat_model, fmt_bytes};
use forja::draw::{rgb, scale, Canvas};
use forja::math::v2;
use forja::platform::{Mouse, Text, CHAR_W, LINE_H};
use forja::project::{Level, SystemSpec};
use forja::world::Lifetime;

use crate::{Action, App, Derived, Focus, Tab, BAD, H, INFO, OK, W, WARN};

pub const BOTTOM_Y: i32 = H as i32 - 300;
pub const ISSUES_X: i32 = 730;
const STATUS_Y: i32 = H as i32 - 22;
const TOP: i32 = 62;
const ROW_H: i32 = 19;

const BG: u32 = rgb(17, 19, 26);
const PANEL: u32 = rgb(24, 27, 36);
const LINE: u32 = 0x2E3340;
const FG: u32 = 0xE5E7EB;
const DIM: u32 = 0x9CA3AF;
const MUTED: u32 = 0x6B7280;
const ACCENT: u32 = 0xFFE14D;
const SEL: u32 = rgb(58, 50, 18);
const HOVER: u32 = rgb(32, 36, 48);
const READ: u32 = 0x3B82F6;
const WRITE: u32 = 0xF97316;

const X_CHK: i32 = 8;
const X_ARW: i32 = 26;
const X_NAME: i32 = 58;
const X_PHASE: i32 = 236;
const X_FREQ: i32 = 318;
const X_POL: i32 = 404;
const X_MS: i32 = 548;
const X_SLACK: i32 = 600;
const MX: i32 = 660;

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

fn chars_for(px: i32) -> usize {
    (px / CHAR_W).max(0) as usize
}

pub struct Ui<'a> {
    c: Canvas<'a>,
    m: Mouse,
    t: &'a mut Vec<Text>,
    pub actions: Vec<Action>,
    tooltip: Option<(i32, i32, String)>,
}

impl<'a> Ui<'a> {
    pub fn new(c: Canvas<'a>, m: Mouse, t: &'a mut Vec<Text>) -> Ui<'a> {
        Ui { c, m, t, actions: Vec::new(), tooltip: None }
    }
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: u32, a: f32) {
        self.c.fill_rect(x0, y0, x1, y1, col, a);
    }
    fn frame(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, col: u32) {
        self.c.rect(x0, y0, x1, y1, col, 1.0);
    }
    fn text(&mut self, x: i32, y: i32, col: u32, s: impl Into<String>) {
        self.t.push(Text::new(x, y, col, s));
    }
    fn hover(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
        self.m.inside(x0, y0, x1, y1)
    }
    fn click(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
        self.m.left_pressed && self.hover(x0, y0, x1, y1)
    }
    fn rclick(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
        self.m.right_pressed && self.hover(x0, y0, x1, y1)
    }
    fn act(&mut self, a: Action) {
        self.actions.push(a);
    }
    /// Botão com largura pelo texto. Retorna (clicado, largura).
    fn button(&mut self, x: i32, y: i32, label: &str, enabled: bool, color: u32) -> (bool, i32) {
        let w = label.chars().count() as i32 * CHAR_W + 16;
        let hov = enabled && self.hover(x, y, x + w, y + 20);
        self.rect(x, y, x + w, y + 20, if hov { rgb(52, 58, 74) } else { rgb(36, 40, 52) }, 1.0);
        self.frame(x, y, x + w, y + 20, if enabled { color } else { LINE });
        self.text(x + 8, y + 3, if enabled { color } else { MUTED }, label);
        (enabled && self.click(x, y, x + w, y + 20), w)
    }
}

pub fn draw(app: &App, d: &Derived, ui: &mut Ui) {
    ui.rect(0, 0, W as i32, H as i32, BG, 1.0);
    top_bar(app, d, ui);
    match app.tab {
        Tab::Pipeline => pipeline(app, d, ui),
        Tab::Grafo => grafo(app, d, ui),
        Tab::Tabelas => tabelas(app, d, ui),
    }
    bottom(app, d, ui);
    status_bar(app, ui);
    if let Some((x, y, s)) = ui.tooltip.take() {
        let w = s.chars().count() as i32 * CHAR_W + 12;
        let x = x.min(W as i32 - w - 4).max(4);
        ui.rect(x, y, x + w, y + 20, rgb(8, 9, 12), 0.95);
        ui.frame(x, y, x + w, y + 20, LINE);
        ui.text(x + 6, y + 3, FG, s);
    }
}

// ------------------------------------------------------------------ barra superior

fn top_bar(app: &App, d: &Derived, ui: &mut Ui) {
    ui.rect(0, 0, W as i32, 28, PANEL, 1.0);
    ui.text(10, 6, ACCENT, "forja-editor");
    let path = app.projeto.display().to_string();
    ui.text(118, 6, FG, fit(&path, 50));
    let (state, col) = if app.dirty() { ("● não salvo", WARN) } else { ("salvo", OK) };
    ui.text(110 + (path.chars().count().min(50) as i32 + 2) * CHAR_W, 6, col, state);

    let running = app.game.is_some();
    let mut x = W as i32 - 10;
    let mut place = |ui: &mut Ui, label: &str, enabled: bool, color: u32, act: Action| {
        let w = label.chars().count() as i32 * CHAR_W + 16;
        x -= w + 6;
        if ui.button(x, 4, label, enabled, color).0 {
            ui.act(act);
        }
    };
    if running {
        place(ui, "Parar", true, BAD, Action::Stop);
    } else {
        place(ui, "Rodar jogo (F5)", d.errors() == 0, OK, Action::Run);
    }
    place(ui, "Medir (F6)", !running && d.errors() == 0, 0x22D3EE, Action::Measure);
    place(ui, "Refazer", true, DIM, Action::Redo);
    place(ui, "Desfazer", true, DIM, Action::Undo);
    place(ui, "Salvar (Ctrl+S)", app.dirty(), FG, Action::Save);

    for (k, (label, tab)) in [("1 Pipeline", Tab::Pipeline), ("2 Grafo", Tab::Grafo), ("3 Tabelas", Tab::Tabelas)].iter().enumerate() {
        let x0 = 10 + k as i32 * 118;
        let active = app.tab == *tab;
        ui.rect(x0, 32, x0 + 112, 54, if active { rgb(46, 52, 68) } else { PANEL }, 1.0);
        if active {
            ui.rect(x0, 52, x0 + 112, 54, ACCENT, 1.0);
        }
        ui.text(x0 + 14, 36, if active { FG } else { DIM }, *label);
        if ui.click(x0, 32, x0 + 112, 54) {
            ui.act(Action::Tab(*tab));
        }
    }
    let info = match &app.profile {
        Some(p) => (format!("perfil: {}  ·  {} tabelas no catálogo", p.origin, p.tables.len()), DIM),
        None => ("sem perfil.forja: aperte Medir (F6) para o jogo exportar o catálogo de tabelas e os tempos".to_string(), WARN),
    };
    ui.text(380, 37, info.1, info.0);
}

fn status_bar(app: &App, ui: &mut Ui) {
    ui.rect(0, STATUS_Y, W as i32, H as i32, PANEL, 1.0);
    ui.text(10, STATUS_Y + 3, app.status_color, fit(&app.status, 100));
    let help = "↑↓ seleciona  1/2/3 abas  Ctrl+Z/Y desfaz/refaz  Ctrl+S salva  F5 roda  F6 mede";
    ui.text(W as i32 - help.chars().count() as i32 * CHAR_W - 10, STATUS_Y + 3, MUTED, help);
}

// ------------------------------------------------------------------ aba Pipeline

fn freq_label(every: u32) -> String {
    if 60 % every == 0 {
        format!("{}Hz", 60 / every)
    } else {
        format!("{:.1}Hz", 60.0 / every as f32)
    }
}

fn pipeline(app: &App, d: &Derived, ui: &mut Ui) {
    let tables = &d.tables;
    let nt = tables.len().max(1) as i32;
    let cw = ((W as i32 - MX - 10) / nt).clamp(22, 48);

    // cabeçalho: nomes das tabelas em duas linhas alternadas
    let hy = TOP;
    for (k, t) in tables.iter().enumerate() {
        let k = k as i32;
        let cx = MX + k * cw + cw / 2;
        let yy = hy + (k % 2) * 15;
        let name = fit(&t.name, chars_for(2 * cw) - 1);
        let col = if t.lifetime == Lifetime::Static { 0x93C5FD } else { DIM };
        ui.text((cx - name.chars().count() as i32 * CHAR_W / 2).max(MX - 30), yy, col, name);
        ui.rect(cx, yy + 15, cx + 1, hy + 36, LINE, 1.0);
    }
    for (x, l) in [(X_CHK, "on"), (X_ARW, "ordem"), (X_NAME + 20, "sistema"), (X_PHASE, "fase"), (X_FREQ, "freq"), (X_POL, "falha"), (X_MS, "ms"), (X_SLACK, "folga")] {
        ui.text(x, hy + 18, MUTED, l);
    }

    let top = hy + 40;
    let bottom = BOTTOM_Y - 24;
    let mut y = top - app.scroll * ROW_H;
    for (gi, g) in app.project.graphs.iter().enumerate() {
        if y >= top && y + ROW_H <= bottom {
            ui.rect(4, y, W as i32 - 4, y + ROW_H, rgb(34, 38, 52), 1.0);
            let cp = d.crit.get(gi);
            let active = g.systems.iter().filter(|s| s.active).count();
            let head = match cp {
                Some(cp) => format!(
                    "grafo '{}' {}   caminho crítico {:.3} ms   trabalho {:.3} ms   paralelismo {:.2}x   {} ativos",
                    g.name,
                    if g.render { "(render, 1x por frame)" } else { "(simulação, tick de 60 Hz)" },
                    cp.span,
                    cp.work,
                    cp.parallelism(),
                    active
                ),
                None => format!("grafo '{}'", g.name),
            };
            ui.text(10, y + 2, ACCENT, head);
        }
        y += ROW_H + 2;
        for s in &g.systems {
            if y >= top && y + ROW_H <= bottom {
                row(app, d, ui, g.render, s, y, cw);
            }
            y += ROW_H;
        }
        y += 6;
    }
    if y > bottom {
        ui.text(W as i32 - 260, bottom + 4, MUTED, "role com a roda do mouse para ver mais");
    }
    ui.text(
        10,
        bottom + 4,
        MUTED,
        "célula: clique vazio → lê → escreve → vazio · botão direito: escolher colunas · fase/falha: clique avança, direito volta",
    );
}

fn row(app: &App, d: &Derived, ui: &mut Ui, render: bool, s: &SystemSpec, y: i32, cw: i32) {
    let name = s.name.as_str();
    let selected = app.sel.as_deref() == Some(name);
    if selected {
        ui.rect(4, y, W as i32 - 4, y + ROW_H, SEL, 1.0);
    } else if ui.hover(4, y, W as i32 - 4, y + ROW_H) {
        ui.rect(4, y, W as i32 - 4, y + ROW_H, HOVER, 1.0);
    }
    let fg = if s.active { FG } else { MUTED };
    let model = d.node_of.get(name).copied();
    let measure = app.profile.as_ref().and_then(|p| p.systems.get(name));

    // ativo
    ui.frame(X_CHK, y + 4, X_CHK + 12, y + 16, DIM);
    if s.active {
        ui.rect(X_CHK + 3, y + 7, X_CHK + 9, y + 13, OK, 1.0);
    }
    if ui.click(X_CHK - 3, y, X_CHK + 15, y + ROW_H) {
        ui.act(Action::ToggleActive(name.to_string()));
    }
    // ordem
    ui.text(X_ARW, y + 2, DIM, "▲");
    ui.text(X_ARW + 14, y + 2, DIM, "▼");
    if ui.click(X_ARW - 2, y, X_ARW + 12, y + ROW_H) {
        ui.act(Action::Move(name.to_string(), -1));
    }
    if ui.click(X_ARW + 12, y, X_ARW + 28, y + ROW_H) {
        ui.act(Action::Move(name.to_string(), 1));
    }
    // nome
    ui.rect(X_NAME, y + 4, X_NAME + 6, y + 15, s.phase.color(), if s.active { 1.0 } else { 0.35 });
    if let Some((g, i)) = model {
        if d.crit[g].on_path(i) && measure.is_some() {
            ui.text(X_NAME + 9, y + 2, ACCENT, "●");
        }
    }
    let ncol = match d.level_of(name) {
        Some(Level::Error) => BAD,
        Some(Level::Warning) if s.active => WARN,
        _ => fg,
    };
    ui.text(X_NAME + 20, y + 2, ncol, fit(name, 22));
    if ui.click(X_NAME, y, X_PHASE - 4, y + ROW_H) {
        ui.act(Action::Select(name.to_string()));
    }
    // fase
    ui.text(X_PHASE, y + 2, fg, s.phase.name());
    if ui.click(X_PHASE, y, X_FREQ - 4, y + ROW_H) {
        ui.act(Action::Phase(name.to_string(), 1));
    }
    if ui.rclick(X_PHASE, y, X_FREQ - 4, y + ROW_H) {
        ui.act(Action::Phase(name.to_string(), -1));
    }
    // frequência
    if render {
        ui.text(X_FREQ + 14, y + 2, MUTED, "frame");
    } else {
        ui.text(X_FREQ, y + 2, DIM, "−");
        let f = freq_label(s.every);
        ui.text(X_FREQ + 38 - f.chars().count() as i32 * CHAR_W / 2, y + 2, fg, f);
        ui.text(X_FREQ + 66, y + 2, DIM, "+");
        if ui.click(X_FREQ - 3, y, X_FREQ + 12, y + ROW_H) {
            ui.act(Action::Every(name.to_string(), -1));
        }
        if ui.click(X_FREQ + 62, y, X_FREQ + 76, y + ROW_H) {
            ui.act(Action::Every(name.to_string(), 1));
        }
    }
    // política
    let pcol = if s.policy.transactional() { 0xC4B5FD } else { fg };
    ui.text(X_POL, y + 2, pcol, fit(s.policy.name(), chars_for(X_MS - X_POL - 6)));
    if ui.click(X_POL, y, X_MS - 4, y + ROW_H) {
        ui.act(Action::Policy(name.to_string(), 1));
    }
    if ui.rclick(X_POL, y, X_MS - 4, y + ROW_H) {
        ui.act(Action::Policy(name.to_string(), -1));
    }
    // medições
    match (model, measure) {
        (Some((g, i)), Some(_)) => {
            ui.text(X_MS, y + 2, fg, format!("{:.3}", d.graphs[g].nodes[i].dur_ms));
            ui.text(X_SLACK, y + 2, DIM, format!("{:.3}", d.crit[g].slack[i]));
        }
        _ => {
            ui.text(X_MS, y + 2, MUTED, "—");
            ui.text(X_SLACK, y + 2, MUTED, "—");
        }
    }
    // matriz
    for (k, t) in d.tables.iter().enumerate() {
        let x0 = MX + k as i32 * cw + 1;
        let x1 = x0 + cw - 2;
        let hov = ui.hover(x0, y, x1, y + ROW_H);
        if hov {
            ui.rect(x0, y + 1, x1, y + ROW_H - 1, 0x3A4152, 1.0);
        }
        let kind = s.access_kind(&t.name);
        if let Some(w) = kind {
            ui.rect(x0, y + 2, x1, y + ROW_H - 2, if w { WRITE } else { READ }, if s.active { 0.9 } else { 0.3 });
            let partial = s.access(&t.name).is_some_and(|a| !a.cols.is_empty());
            let label = format!("{}{}", if w { "W" } else { "R" }, if partial { "*" } else { "" });
            ui.text(x0 + cw / 2 - label.len() as i32 * CHAR_W / 2 - 1, y + 2, 0xFFFFFF, label);
            if let Some(m) = measure {
                if m.runs > 0 && s.active && !m.touched.contains(&t.name) {
                    ui.rect(x1 - 6, y + 3, x1 - 2, y + 7, BAD, 1.0);
                }
            }
        }
        if ui.click(x0, y, x1, y + ROW_H) {
            ui.act(Action::Access(name.to_string(), t.name.clone()));
        }
        if kind.is_some() && ui.rclick(x0, y, x1, y + ROW_H) {
            ui.act(Action::OpenCols(name.to_string(), t.name.clone()));
        }
        if hov {
            let what = match kind {
                Some(true) => "escreve",
                Some(false) => "lê",
                None => "não acessa",
            };
            let cols = s.access(&t.name).map(|a| if a.cols.is_empty() { "todas as colunas".to_string() } else { a.cols.join(", ") }).unwrap_or_default();
            let util = s
                .access(&t.name)
                .and_then(|a| access_stat_model(t, &a.cols))
                .map(|st| format!(" · uso de cache {:.0}%", st.util() * 100.0))
                .unwrap_or_default();
            ui.tooltip = Some((ui.m.x + 12, y + ROW_H + 2, format!("{} {} {} {}{}", name, what, t.name, cols, util)));
        }
    }
}

// ------------------------------------------------------------------ aba Grafo

fn grafo(app: &App, d: &Derived, ui: &mut Ui) {
    let ng = d.graphs.len().max(1) as i32;
    let total = BOTTOM_Y - 6 - TOP;
    let gh = total / ng;
    for (gi, g) in d.graphs.iter().enumerate() {
        let y0 = TOP + gi as i32 * gh;
        let y1 = y0 + gh - 6;
        ui.rect(4, y0, W as i32 - 4, y1, rgb(20, 23, 31), 1.0);
        let cp = &d.crit[gi];
        ui.text(
            12,
            y0 + 4,
            ACCENT,
            format!(
                "grafo '{}'   caminho crítico {:.3} ms   paralelismo {:.2}x   (seta = conflito de dados; amarelo = caminho crítico; +x = folga em ms; selecione para ver as tabelas)",
                g.name,
                cp.span,
                cp.parallelism()
            ),
        );
        if g.nodes.is_empty() {
            continue;
        }
        let lv = g.levels();
        let nl = lv.iter().max().copied().unwrap_or(0) as i32 + 1;
        let col_w = ((W as i32 - 30) / nl).min(220);
        let box_w = col_w - 14;
        let box_h = 34;
        let mut stack = vec![0i32; nl as usize];
        let mut pos = vec![(0i32, 0i32); g.nodes.len()];
        for (i, &l) in lv.iter().enumerate() {
            pos[i] = (16 + l as i32 * col_w, y0 + 26 + stack[l] * (box_h + 12));
            stack[l] += 1;
        }
        let sel = app.sel.as_deref().and_then(|s| g.nodes.iter().position(|n| n.name == s));
        let crit_edge = |i: usize, j: usize| cp.path.windows(2).any(|w| w[0] == i && w[1] == j);
        for (i, j) in g.reduced_edges() {
            let a = v2((pos[i].0 + box_w) as f32, (pos[i].1 + box_h / 2) as f32);
            let b = v2(pos[j].0 as f32 - 2.0, (pos[j].1 + box_h / 2) as f32);
            let touches = sel == Some(i) || sel == Some(j);
            let col = if crit_edge(i, j) {
                ACCENT
            } else if touches {
                0xE5E7EB
            } else {
                0x4B5563
            };
            ui.c.line(a, b, col, 1.0);
            ui.c.line(a + v2(0.0, 1.0), b + v2(0.0, 1.0), col, 0.7);
            let dir = (b - a).norm();
            let side = dir.perp();
            ui.c.poly(&[b, b - dir * 8.0 + side * 4.0, b - dir * 8.0 - side * 4.0], col, 1.0);
        }
        for (i, n) in g.nodes.iter().enumerate() {
            let (x, y) = pos[i];
            let on = cp.on_path(i);
            ui.rect(x, y, x + box_w, y + box_h, scale(n.phase.color(), 0.32), 1.0);
            ui.rect(x, y, x + 4, y + box_h, n.phase.color(), 1.0);
            let border = match d.level_of(&n.name) {
                Some(Level::Error) => BAD,
                _ if sel == Some(i) => 0xFFFFFF,
                _ if on => ACCENT,
                _ => scale(n.phase.color(), 0.7),
            };
            ui.frame(x, y, x + box_w, y + box_h, border);
            if sel == Some(i) {
                ui.frame(x - 1, y - 1, x + box_w + 1, y + box_h + 1, border);
            }
            ui.text(x + 8, y + 2, FG, fit(&n.name, chars_for(box_w - 10)));
            let sub = if app.profile.as_ref().is_some_and(|p| p.systems.contains_key(&n.name)) {
                if cp.slack[i] > 0.0005 {
                    format!("{:.3}ms +{:.3}", n.dur_ms, cp.slack[i])
                } else {
                    format!("{:.3} ms", n.dur_ms)
                }
            } else {
                format!("{} · sem medição", n.phase.name())
            };
            ui.text(x + 8, y + 17, DIM, fit(&sub, chars_for(box_w - 10)));
            if ui.click(x, y, x + box_w, y + box_h) {
                ui.act(Action::Select(n.name.clone()));
            }
        }
        if let Some(gs) = app.project.graphs.get(gi) {
            let off: Vec<&str> = gs.systems.iter().filter(|s| !s.active).map(|s| s.name.as_str()).collect();
            if !off.is_empty() {
                ui.text(12, y1 - 18, MUTED, format!("desativados (fora do grafo): {}", off.join(", ")));
            }
        }
    }
}

// ------------------------------------------------------------------ aba Tabelas

fn tabelas(app: &App, d: &Derived, ui: &mut Ui) {
    let mut y = TOP;
    if d.synthetic {
        ui.text(10, y, WARN, "Sem catálogo: estes nomes vêm do projeto. Aperte Medir (F6) para o jogo exportar as tabelas reais (colunas, bytes, linhas).");
        y += 20;
    }
    ui.text(
        10,
        y,
        MUTED,
        format!(
            "{:<16} {:<11} {:<4} {:>9} {:>6} {:>4} {:>9} {:>2} {:>2} {:>4} {:>5}  {}",
            "tabela", "vida", "lay", "linhas", "B/lin", "pad", "total", "W", "R", "inv", "uso", "nota (layout e colunas vêm do código: corrida/src/tables.rs)"
        ),
    );
    y += LINE_H + 4;
    let active = || app.project.graphs.iter().flat_map(|g| g.systems.iter()).filter(|s| s.active);
    for t in &d.tables {
        if y + LINE_H > BOTTOM_Y - 4 {
            break;
        }
        let selected = app.sel_table.as_deref() == Some(t.name.as_str());
        if selected {
            ui.rect(4, y - 1, W as i32 - 4, y + LINE_H, SEL, 1.0);
        } else if ui.hover(4, y - 1, W as i32 - 4, y + LINE_H) {
            ui.rect(4, y - 1, W as i32 - 4, y + LINE_H, HOVER, 1.0);
        }
        let writers = active().filter(|s| s.access_kind(&t.name) == Some(true)).count();
        let readers = active().filter(|s| s.access_kind(&t.name) == Some(false)).count();
        let worst = active()
            .filter_map(|s| s.access(&t.name))
            .filter_map(|a| access_stat_model(t, &a.cols))
            .map(|st| st.util())
            .fold(f64::NAN, f64::min);
        let uso = if worst.is_nan() { "-".to_string() } else { format!("{:.0}%", worst * 100.0) };
        let line = format!(
            "{:<16} {:<11} {:<4} {:>9} {:>6} {:>4} {:>9} {:>2} {:>2} {:>4} {:>5}  {}",
            fit(&t.name, 16),
            t.lifetime.name(),
            format!("{:?}", t.layout),
            t.rows,
            t.row_bytes(),
            t.padding(),
            fmt_bytes(t.total_bytes()),
            writers,
            readers,
            t.invariants.len(),
            uso,
            t.note
        );
        let col = if !worst.is_nan() && worst < 0.5 { BAD } else { FG };
        ui.text(10, y, col, fit(&line, chars_for(W as i32 - 20)));
        if ui.click(4, y - 1, W as i32 - 4, y + LINE_H) {
            ui.act(Action::SelectTable(t.name.clone()));
        }
        y += LINE_H + 1;
    }
    let total: usize = d.tables.iter().map(|t| t.total_bytes()).sum();
    ui.text(10, BOTTOM_Y - 22, ACCENT, format!("Memória declarada: {}   ·   clique numa tabela para ver quem usa cada coluna", fmt_bytes(total)));
}

// ------------------------------------------------------------------ painel inferior

fn bottom(app: &App, d: &Derived, ui: &mut Ui) {
    ui.rect(0, BOTTOM_Y, W as i32, STATUS_Y, PANEL, 1.0);
    ui.rect(0, BOTTOM_Y, W as i32, BOTTOM_Y + 1, LINE, 1.0);
    ui.rect(ISSUES_X, BOTTOM_Y, ISSUES_X + 1, STATUS_Y, LINE, 1.0);
    match &app.focus {
        Focus::Note { system, buf } => note_editor(ui, system, buf),
        Focus::Cols { system, table } => cols_chooser(app, d, ui, system, table),
        Focus::None => match (app.tab, &app.sel_table, &app.sel) {
            (Tab::Tabelas, Some(t), _) => table_details(app, d, ui, t),
            (_, _, Some(s)) if app.project.find(s).is_some() => system_details(app, d, ui, s),
            _ => help(ui),
        },
    }
    issues(app, d, ui);
}

struct Lines {
    x: i32,
    y: i32,
    max_y: i32,
    width: usize,
}

impl Lines {
    fn put(&mut self, ui: &mut Ui, col: u32, s: impl AsRef<str>) -> i32 {
        let y = self.y;
        if self.y + LINE_H <= self.max_y {
            ui.text(self.x, self.y, col, fit(s.as_ref(), self.width));
        }
        self.y += LINE_H;
        y
    }
}

fn panel_lines() -> Lines {
    Lines { x: 12, y: BOTTOM_Y + 8, max_y: STATUS_Y - 4, width: chars_for(ISSUES_X - 24) }
}

fn help(ui: &mut Ui) {
    let mut l = panel_lines();
    l.put(ui, ACCENT, "Como usar");
    for s in [
        "Cada linha é um sistema (uma função no código). Cada coluna da direita é uma tabela do jogo.",
        "Célula: clique alterna entre não acessa → lê (R) → escreve (W). Botão direito: escolher colunas (R*).",
        "O grafo sai dessas células: dois sistemas que tocam a mesma tabela, e um deles escreve, rodam em ordem.",
        "Fase e falha: clique avança, botão direito volta. Frequência: − e +. Quadrado à esquerda: ativa/desativa.",
        "▲▼ muda a ordem dentro da mesma fase. A ordem decide quem roda primeiro quando há conflito.",
        "À direita, a validação (a mesma que o jogo faz ao iniciar) e as sugestões da análise, ao vivo.",
        "Medir (F6) roda o jogo headless e traz tempos reais e o que cada sistema acessou de fato.",
        "Rodar (F5) salva e abre o jogo com este pipeline. Ponto vermelho na célula = declarado mas não usado.",
    ] {
        l.put(ui, DIM, s);
    }
}

fn note_editor(ui: &mut Ui, system: &str, buf: &str) {
    let mut l = panel_lines();
    l.put(ui, ACCENT, format!("Nota de {}", system));
    let y = l.y + 4;
    ui.rect(12, y, ISSUES_X - 14, y + 24, BG, 1.0);
    ui.frame(12, y, ISSUES_X - 14, y + 24, ACCENT);
    ui.text(18, y + 5, FG, format!("{}_", fit(buf, chars_for(ISSUES_X - 50))));
    ui.text(12, y + 32, DIM, "Enter confirma · Esc cancela");
}

fn cols_chooser(app: &App, d: &Derived, ui: &mut Ui, system: &str, table: &str) {
    let mut l = panel_lines();
    let Some((g, i)) = app.project.find(system) else { return };
    let s = &app.project.graphs[g].systems[i];
    let Some(acc) = s.access(table) else {
        l.put(ui, DIM, "o sistema não acessa mais esta tabela (Esc fecha)");
        return;
    };
    let verb = if s.access_kind(table) == Some(true) { "escreve" } else { "lê" };
    l.put(ui, ACCENT, format!("Colunas de {} que {} {}", table, system, verb));
    l.put(ui, DIM, "Clique para marcar/desmarcar. Declarar só o que usa melhora a estimativa de cache e documenta o acesso.");
    let Some(t) = d.tables.iter().find(|t| t.name == table) else { return };
    if t.columns.is_empty() {
        l.put(ui, WARN, "sem catálogo de colunas: aperte Medir (F6)");
        return;
    }
    let (clicked, _) = ui.button(12, l.y + 2, "todas as colunas", !acc.cols.is_empty(), FG);
    if clicked {
        ui.act(Action::AllCols(system.to_string(), table.to_string()));
    }
    if ui.button(150, l.y + 2, "fechar (Esc)", true, DIM).0 {
        ui.act(Action::CloseFocus);
    }
    let base = l.y + 30;
    for (k, (col, bytes)) in t.columns.iter().enumerate() {
        let x = 16 + (k as i32 / 9) * 230;
        let y = base + (k as i32 % 9) * 18;
        let on = acc.cols.is_empty() || acc.cols.contains(col);
        ui.frame(x, y + 2, x + 12, y + 14, DIM);
        if on {
            ui.rect(x + 3, y + 5, x + 9, y + 11, if verb == "lê" { READ } else { WRITE }, 1.0);
        }
        ui.text(x + 18, y, if on { FG } else { MUTED }, format!("{:<12} {:>3} B", fit(col, 12), bytes));
        if ui.click(x, y, x + 210, y + 17) {
            ui.act(Action::ToggleCol(system.to_string(), table.to_string(), col.clone()));
        }
    }
    let y = base + 9 * 18 + 4;
    if let Some(st) = access_stat_model(t, &acc.cols) {
        let col = if st.util() < 0.5 { BAD } else { OK };
        ui.text(
            12,
            y,
            col,
            format!("varrer {} linhas: {} úteis de {} trazidos ao cache ({:.0}%) · layout {:?}", t.rows, fmt_bytes(st.useful), fmt_bytes(st.loaded), st.util() * 100.0, t.layout),
        );
        if st.util() < 0.5 {
            ui.text(12, y + LINE_H, DIM, "AoS: a linha inteira vem para o cache mesmo lendo poucas colunas. Separar as colunas quentes resolve.");
        }
    }
}

fn system_details(app: &App, d: &Derived, ui: &mut Ui, name: &str) {
    let mut l = panel_lines();
    let Some((g, i)) = app.project.find(name) else { return };
    let gs = &app.project.graphs[g];
    let s = &gs.systems[i];
    let freq = if gs.render { "1x por frame".to_string() } else { freq_label(s.every) };
    l.put(
        ui,
        ACCENT,
        format!("{}   [{}]   {}   {}{}", s.name, s.phase.name(), s.policy.name(), freq, if s.active { "" } else { "   DESATIVADO" }),
    );
    let ny = l.put(ui, FG, format!("nota: {}", if s.note.is_empty() { "(vazia)" } else { &s.note }));
    if ui.button(ISSUES_X - 80, ny - 2, "editar", true, DIM).0 {
        ui.act(Action::EditNote(name.to_string()));
    }
    match d.node_of.get(name) {
        Some(&(gm, ni)) => {
            let gmod = &d.graphs[gm];
            let deps: Vec<String> = gmod.deps[ni].iter().map(|&j| format!("{} ({})", gmod.nodes[j].name, gmod.conflict_tables(j, ni, &d.tables).join("/"))).collect();
            let outs: Vec<&str> = gmod.dependents[ni].iter().map(|&j| gmod.nodes[j].name.as_str()).collect();
            l.put(ui, DIM, format!("espera por: {}", if deps.is_empty() { "nada (raiz do grafo)".into() } else { deps.join(", ") }));
            l.put(ui, DIM, format!("libera: {}", if outs.is_empty() { "nada (folha)".into() } else { outs.join(", ") }));
            let txn = gmod.txn_bytes(ni, &d.tables);
            if txn > 0 {
                l.put(ui, 0xC4B5FD, format!("transação: copia {} por execução para poder desfazer em caso de falha", fmt_bytes(txn)));
            }
        }
        None => {
            l.put(ui, MUTED, "fora do grafo (desativado ou com erro)");
        }
    }
    let measure = app.profile.as_ref().and_then(|p| p.systems.get(name));
    match measure {
        Some(m) => l.put(ui, FG, format!("última medição: {:.4} ms em média · {} execuções · {} falhas", m.ms, m.runs, m.failures)),
        None => l.put(ui, MUTED, "sem medição: aperte Medir (F6)"),
    };
    l.put(ui, ACCENT, "acessos declarados");
    for (a, w) in s.writes.iter().map(|a| (a, true)).chain(s.reads.iter().map(|a| (a, false))) {
        let cols = if a.cols.is_empty() { "todas".to_string() } else { a.cols.join(",") };
        let stat = d
            .tables
            .iter()
            .find(|t| t.name == a.table)
            .and_then(|t| access_stat_model(t, &a.cols))
            .map(|st| format!("{} úteis de {} ({:.0}%)", fmt_bytes(st.useful), fmt_bytes(st.loaded), st.util() * 100.0))
            .unwrap_or_else(|| "acesso esparso".into());
        let (used, col) = match measure {
            Some(m) if m.runs > 0 && !m.touched.contains(&a.table) => ("NÃO usado na última execução", WARN),
            Some(m) if m.runs > 0 => ("usado", OK),
            _ => ("", DIM),
        };
        l.put(ui, col, format!("  {} {:<15} {:<22} {:<30} {}", if w { "W" } else { "R" }, fit(&a.table, 15), fit(&cols, 22), stat, used));
    }
    for is in d.issues.iter().filter(|x| x.system.as_deref() == Some(name)) {
        let col = match is.level {
            Level::Error => BAD,
            Level::Warning => WARN,
            Level::Info => INFO,
        };
        l.put(ui, col, &is.msg);
    }
}

fn table_details(app: &App, d: &Derived, ui: &mut Ui, name: &str) {
    let mut l = panel_lines();
    let Some(t) = d.tables.iter().find(|t| t.name == name) else { return };
    l.put(
        ui,
        ACCENT,
        format!("{}   {}   {:?}   {} linhas   {} B/linha   {}", t.name, t.lifetime.name(), t.layout, t.rows, t.row_bytes(), fmt_bytes(t.total_bytes())),
    );
    if !t.invariants.is_empty() {
        l.put(ui, DIM, format!("invariantes: {}", t.invariants.join("; ")));
    }
    l.put(ui, DIM, "Quem usa cada coluna (sistemas ativos). Colunas usadas por muitos = quentes; por poucos = frias.");
    let active: Vec<&SystemSpec> = app.project.graphs.iter().flat_map(|g| g.systems.iter()).filter(|s| s.active).collect();
    let n_users = |col: &str| -> Vec<(&str, bool)> {
        active
            .iter()
            .filter_map(|s| s.access(&t.name).map(|a| (s, a)))
            .filter(|(_, a)| a.cols.is_empty() || a.cols.iter().any(|c| c == col))
            .map(|(s, _)| (s.name.as_str(), s.access_kind(&t.name) == Some(true)))
            .collect()
    };
    let max = t.columns.iter().map(|(c, _)| n_users(c).len()).max().unwrap_or(1).max(1);
    for (col, bytes) in &t.columns {
        let users = n_users(col);
        let y = l.y;
        let bar = (users.len() as f32 / max as f32 * 120.0) as i32;
        ui.rect(190, y + 3, 190 + bar, y + 12, if users.is_empty() { MUTED } else { READ }, 0.8);
        let names: Vec<String> = users.iter().map(|(n, w)| format!("{}{}", n, if *w { "(W)" } else { "" })).collect();
        l.put(ui, if users.is_empty() { MUTED } else { FG }, format!("{:<14} {:>3} B", fit(col, 14), bytes));
        ui.text(320, y, DIM, fit(&format!("{}  {}", users.len(), names.join(", ")), chars_for(ISSUES_X - 330)));
    }
    if t.columns.is_empty() {
        l.put(ui, WARN, "sem catálogo de colunas: aperte Medir (F6)");
    }
}

fn issues(app: &App, d: &Derived, ui: &mut Ui) {
    let x = ISSUES_X + 12;
    let mut y = BOTTOM_Y + 8;
    let count = |lv: Level| d.issues.iter().filter(|i| i.level == lv).count();
    let (e, w, i) = (count(Level::Error), count(Level::Warning), count(Level::Info));
    ui.text(
        x,
        y,
        if e > 0 { BAD } else { OK },
        format!("Validação: {} erro(s) · {} aviso(s) · {} info (a mesma do jogo)", e, w, i),
    );
    y += LINE_H + 4;
    let width = chars_for(W as i32 - x - 16);
    let mut lines: Vec<(u32, String, Option<String>)> = Vec::new();
    for is in &d.issues {
        let (col, mark) = match is.level {
            Level::Error => (BAD, "✖"),
            Level::Warning => (WARN, "!"),
            Level::Info => (INFO, "·"),
        };
        lines.push((col, format!("{} {}", mark, is.msg), is.system.clone()));
    }
    if d.issues.is_empty() {
        lines.push((OK, "nenhum problema: o jogo aceita este pipeline".into(), None));
    }
    lines.push((ACCENT, "Sugestões da análise (tempos do último perfil):".into(), None));
    for s in &d.insights {
        lines.push((DIM, format!("• {}", s), None));
    }
    let room = ((STATUS_Y - 4 - y) / LINE_H).max(0) as usize;
    let start = app.issue_scroll.min(lines.len().saturating_sub(room));
    for (col, s, sys) in lines.iter().skip(start).take(room) {
        if let Some(sys) = sys {
            if ui.hover(x - 4, y, W as i32 - 4, y + LINE_H) {
                ui.rect(x - 4, y, W as i32 - 4, y + LINE_H, HOVER, 1.0);
            }
            if ui.click(x - 4, y, W as i32 - 4, y + LINE_H) {
                ui.act(Action::Select(sys.clone()));
            }
        }
        ui.text(x, y, *col, fit(s, width));
        y += LINE_H;
    }
    if lines.len() > room {
        let s = format!("{}–{} de {} · role", start + 1, (start + room).min(lines.len()), lines.len());
        ui.text(W as i32 - 12 - s.chars().count() as i32 * CHAR_W, BOTTOM_Y + 8, MUTED, s);
    }
}
