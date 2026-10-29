//! forja-editor: o pipeline do projeto como uma tabela editável, antes de rodar o jogo.
//!
//!   cargo run --release -p forja-editor                       abre corrida/projeto.forja
//!   cargo run --release -p forja-editor -- outro/projeto.forja
//!
//! Lê `projeto.forja` (o que você decide) e `perfil.forja` (o que o jogo reportou na última
//! execução: catálogo de tabelas, tempos, acessos reais). Toda edição é validada ao vivo com a
//! mesma função que o jogo usa ao iniciar; F5 roda o jogo, F6 mede headless e atualiza os tempos.
//!
//! `--shot arquivo.png [--aba grafo|tabelas] [--sel Sistema] [--cols Sistema:Tabela] [--tabela T]`
//! desenha um frame direto num PNG, sem abrir janela (documentação e testes).

#![windows_subsystem = "windows"]

mod views;

use std::collections::HashMap;
use std::path::PathBuf;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use forja::analysis::{self, CritPath, GraphModel, TableModel};
use forja::draw::Canvas;
use forja::platform::{self, Keys, Text, Window};
use forja::project::{self, Issue, Level, Profile, Project, SystemSpec};
use forja::schedule::{FailPolicy, Phase};
use forja::world::{Layout, Lifetime};

pub const W: usize = 1440;
pub const H: usize = 860;

pub const OK: u32 = 0x4ADE80;
pub const WARN: u32 = 0xFACC15;
pub const BAD: u32 = 0xF87171;
pub const INFO: u32 = 0x9CA3AF;

const VK_BACK: u8 = 0x08;
const VK_RETURN: u8 = 0x0D;
const VK_CONTROL: u8 = 0x11;
const EVERY_STEPS: [u32; 12] = [1, 2, 3, 4, 5, 6, 10, 12, 15, 20, 30, 60];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Pipeline,
    Grafo,
    Tabelas,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Focus {
    None,
    Note { system: String, buf: String },
    Cols { system: String, table: String },
}

#[derive(Clone, Debug)]
pub enum Action {
    Select(String),
    SelectTable(String),
    ToggleActive(String),
    Move(String, i32),
    Phase(String, i32),
    Every(String, i32),
    Policy(String, i32),
    Access(String, String),
    OpenCols(String, String),
    ToggleCol(String, String, String),
    AllCols(String, String),
    EditNote(String),
    Tab(Tab),
    Save,
    Undo,
    Redo,
    Run,
    Measure,
    Stop,
    Reload,
    CloseFocus,
}

/// Tudo que é calculado a partir do projeto + perfil a cada frame.
pub struct Derived {
    pub tables: Vec<TableModel>,
    pub synthetic: bool,
    pub graphs: Vec<GraphModel>,
    pub crit: Vec<CritPath>,
    pub issues: Vec<Issue>,
    pub insights: Vec<String>,
    /// nome do sistema -> (grafo, nó no modelo); só sistemas ativos.
    pub node_of: HashMap<String, (usize, usize)>,
}

impl Derived {
    pub fn level_of(&self, system: &str) -> Option<Level> {
        self.issues.iter().filter(|i| i.system.as_deref() == Some(system)).map(|i| i.level).min()
    }
    pub fn errors(&self) -> usize {
        self.issues.iter().filter(|i| i.level == Level::Error).count()
    }
}

pub struct App {
    pub projeto: PathBuf,
    pub perfil: PathBuf,
    pub jogo: PathBuf,
    pub project: Project,
    saved: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
    pub profile: Option<Profile>,
    profile_mtime: Option<SystemTime>,
    project_mtime: Option<SystemTime>,
    pub tab: Tab,
    pub sel: Option<String>,
    pub sel_table: Option<String>,
    pub focus: Focus,
    pub status: String,
    pub status_color: u32,
    pub game: Option<(Child, &'static str)>,
    pub scroll: i32,
    pub issue_scroll: usize,
    last_poll: Instant,
}

fn mtime(p: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn sys_mut<'a>(p: &'a mut Project, name: &str) -> Result<&'a mut SystemSpec, String> {
    let (g, i) = p.find(name).ok_or_else(|| format!("sistema '{}' não encontrado", name))?;
    Ok(&mut p.graphs[g].systems[i])
}

impl App {
    fn new(projeto: PathBuf) -> App {
        let perfil = projeto.with_file_name("perfil.forja");
        let jogo = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("corrida.exe"))).unwrap_or_else(|| PathBuf::from("corrida.exe"));
        let (project, status, color) = match Project::load(&projeto) {
            Ok(p) => (p, format!("{} carregado", projeto.display()), OK),
            Err(e) => (Project::default(), format!("erro lendo o projeto: {}", e.join("; ")), BAD),
        };
        let mut app = App {
            projeto: projeto.clone(),
            perfil,
            jogo,
            saved: project.clone(),
            project,
            undo: Vec::new(),
            redo: Vec::new(),
            profile: None,
            profile_mtime: None,
            project_mtime: mtime(&projeto),
            tab: Tab::Pipeline,
            sel: None,
            sel_table: None,
            focus: Focus::None,
            status,
            status_color: color,
            game: None,
            scroll: 0,
            issue_scroll: 0,
            last_poll: Instant::now(),
        };
        app.reload_profile(false);
        app
    }

    pub fn dirty(&self) -> bool {
        self.project != self.saved
    }

    pub fn set_status(&mut self, s: impl Into<String>, color: u32) {
        self.status = s.into();
        self.status_color = color;
    }

    fn reload_profile(&mut self, announce: bool) {
        self.profile_mtime = mtime(&self.perfil);
        match project::load_profile(&self.perfil) {
            Ok(p) => {
                if announce {
                    self.set_status(format!("perfil atualizado ({})", p.origin), OK);
                }
                self.profile = Some(p);
            }
            Err(_) => self.profile = None,
        }
    }

    pub fn derive(&self) -> Derived {
        let (tables, synthetic) = match &self.profile {
            Some(p) if !p.tables.is_empty() => (p.tables.clone(), false),
            _ => (
                self.project
                    .table_names()
                    .into_iter()
                    .map(|name| TableModel {
                        name,
                        lifetime: Lifetime::Session,
                        layout: Layout::AoS,
                        row_size: 0,
                        columns: Vec::new(),
                        rows: 0,
                        sparse: true,
                        external: false,
                        note: "sem catálogo: rode o jogo uma vez".into(),
                        invariants: Vec::new(),
                    })
                    .collect(),
                true,
            ),
        };
        let dur = self.profile.as_ref().map(|p| p.durations()).unwrap_or_default();
        let graphs: Vec<GraphModel> = self.project.graphs.iter().map(|g| project::graph_model(g, &tables, &dur)).collect();
        let crit = graphs.iter().map(|g| g.critical_path()).collect();
        let implemented = self.profile.as_ref().map(|p| p.implemented.clone()).unwrap_or_default();
        let mut issues = project::validate(&self.project, if synthetic { &[] } else { &tables }, &implemented);
        if let Some(p) = &self.profile {
            for s in self.project.graphs.iter().flat_map(|g| g.systems.iter()).filter(|s| s.active) {
                let Some(m) = p.systems.get(&s.name) else { continue };
                if m.runs == 0 {
                    continue;
                }
                for a in s.reads.iter().chain(s.writes.iter()) {
                    if !m.touched.contains(&a.table) {
                        issues.push(Issue {
                            level: Level::Warning,
                            system: Some(s.name.clone()),
                            msg: format!("{}: declara {} mas não acessou na última execução", s.name, a.table),
                        });
                    }
                }
            }
            issues.sort_by_key(|i| i.level);
        }
        let insights = analysis::insights_model(&tables, &graphs);
        let mut node_of = HashMap::new();
        for (gi, g) in graphs.iter().enumerate() {
            for (ni, n) in g.nodes.iter().enumerate() {
                node_of.insert(n.name.clone(), (gi, ni));
            }
        }
        Derived { tables, synthetic, graphs, crit, issues, insights, node_of }
    }

    /// Aplica uma edição com desfazer. Um Err desfaz a tentativa e vira mensagem de status.
    fn edit(&mut self, f: impl FnOnce(&mut Project) -> Result<(), String>) {
        let before = self.project.clone();
        match f(&mut self.project) {
            Ok(()) => {
                self.project.normalize();
                if self.project != before {
                    self.undo.push(before);
                    if self.undo.len() > 300 {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                }
            }
            Err(m) => {
                self.project = before;
                self.set_status(m, WARN);
            }
        }
    }

    fn save(&mut self) -> bool {
        match self.project.save(&self.projeto) {
            Ok(()) => {
                self.saved = self.project.clone();
                self.project_mtime = mtime(&self.projeto);
                self.set_status(format!("salvo em {}", self.projeto.display()), OK);
                true
            }
            Err(e) => {
                self.set_status(format!("falha ao salvar: {}", e), BAD);
                false
            }
        }
    }

    fn launch(&mut self, measure: bool, d: &Derived) {
        if self.game.is_some() {
            self.set_status("o jogo já está rodando", WARN);
            return;
        }
        if d.errors() > 0 {
            self.set_status(format!("corrija os {} erro(s) em vermelho antes de rodar: o jogo recusaria este pipeline", d.errors()), BAD);
            return;
        }
        if self.dirty() && !self.save() {
            return;
        }
        if !self.jogo.exists() {
            self.set_status(format!("não achei o jogo em {}: rode `cargo build --release -p corrida`", self.jogo.display()), BAD);
            return;
        }
        let projeto = std::path::absolute(&self.projeto).unwrap_or_else(|_| self.projeto.clone());
        let mut cmd = Command::new(&self.jogo);
        cmd.arg("--projeto").arg(&projeto);
        if measure {
            cmd.arg("--analyze").arg("1800");
        }
        // sem console extra; a saída do jogo vai para reports/jogo.log
        cmd.creation_flags(0x0800_0000);
        let _ = std::fs::create_dir_all("reports");
        if let Ok(log) = std::fs::File::create("reports/jogo.log") {
            if let Ok(log2) = log.try_clone() {
                cmd.stdout(Stdio::from(log)).stderr(Stdio::from(log2));
            }
        }
        match cmd.spawn() {
            Ok(child) => {
                let kind = if measure { "medição" } else { "jogo" };
                self.set_status(if measure { "medindo: 1800 ticks headless...".to_string() } else { "jogo rodando: feche a janela do jogo para voltar".to_string() }, WARN);
                self.game = Some((child, kind));
            }
            Err(e) => self.set_status(format!("não consegui iniciar o jogo: {}", e), BAD),
        }
    }

    fn poll(&mut self) {
        if self.last_poll.elapsed() < Duration::from_millis(400) {
            return;
        }
        self.last_poll = Instant::now();
        if let Some((child, kind)) = &mut self.game {
            if let Ok(Some(st)) = child.try_wait() {
                let kind = *kind;
                self.game = None;
                self.reload_profile(false);
                let (msg, color) = match st.code() {
                    Some(0) => (format!("{} terminou; tempos e acessos atualizados a partir do perfil", kind), OK),
                    Some(2) => ("o jogo recusou o pipeline: veja reports/jogo.log".to_string(), BAD),
                    Some(3) => ("o jogo abortou por falha crítica: veja a pasta reports/".to_string(), BAD),
                    other => (format!("{} terminou com código {:?}", kind, other), WARN),
                };
                self.set_status(msg, color);
            }
        }
        if mtime(&self.perfil) != self.profile_mtime {
            self.reload_profile(self.game.is_none());
        }
        let pm = mtime(&self.projeto);
        if pm != self.project_mtime {
            self.project_mtime = pm;
            if self.dirty() {
                self.set_status("projeto.forja mudou no disco: Ctrl+S sobrescreve, Ctrl+R recarrega", WARN);
            } else if let Ok(p) = Project::load(&self.projeto) {
                self.project = p.clone();
                self.saved = p;
                self.set_status("projeto.forja mudou no disco e foi recarregado", OK);
            }
        }
    }

    fn handle_keys(&mut self, keys: &Keys, chars: &[char]) -> Vec<Action> {
        let mut out = Vec::new();
        if let Focus::Note { system, buf } = &mut self.focus {
            buf.extend(chars.iter());
            if keys.pressed(VK_BACK) {
                buf.pop();
            }
            if keys.pressed(VK_RETURN) {
                out.push(Action::EditNote(system.clone()));
            }
            if keys.pressed(platform::vk::ESC) {
                out.push(Action::CloseFocus);
            }
            return out;
        }
        let ctrl = keys.down(VK_CONTROL);
        if ctrl && keys.pressed(b'S') {
            out.push(Action::Save);
        }
        if ctrl && keys.pressed(b'Z') {
            out.push(Action::Undo);
        }
        if ctrl && keys.pressed(b'Y') {
            out.push(Action::Redo);
        }
        if ctrl && keys.pressed(b'R') {
            out.push(Action::Reload);
        }
        if keys.pressed(platform::vk::F5) {
            out.push(Action::Run);
        }
        if keys.pressed(platform::vk::F6) {
            out.push(Action::Measure);
        }
        if keys.pressed(platform::vk::ESC) {
            out.push(Action::CloseFocus);
        }
        for (k, t) in [(b'1', Tab::Pipeline), (b'2', Tab::Grafo), (b'3', Tab::Tabelas)] {
            if !ctrl && keys.pressed(k) {
                out.push(Action::Tab(t));
            }
        }
        if keys.pressed(platform::vk::UP) || keys.pressed(platform::vk::DOWN) {
            let names: Vec<String> = self.project.graphs.iter().flat_map(|g| g.systems.iter().map(|s| s.name.clone())).collect();
            if !names.is_empty() {
                let cur = self.sel.as_ref().and_then(|s| names.iter().position(|n| n == s));
                let next = match (cur, keys.pressed(platform::vk::DOWN)) {
                    (None, _) => 0,
                    (Some(i), true) => (i + 1).min(names.len() - 1),
                    (Some(i), false) => i.saturating_sub(1),
                };
                out.push(Action::Select(names[next].clone()));
            }
        }
        out
    }

    fn apply(&mut self, a: Action, d: &Derived) {
        match a {
            Action::Select(n) => {
                if !matches!(&self.focus, Focus::Cols { system, .. } if *system == n) {
                    self.focus = Focus::None;
                }
                self.sel = Some(n);
            }
            Action::SelectTable(t) => self.sel_table = Some(t),
            Action::ToggleActive(n) => self.edit(|p| {
                let s = sys_mut(p, &n)?;
                s.active = !s.active;
                Ok(())
            }),
            Action::Move(n, dir) => self.edit(|p| {
                let (g, i) = p.find(&n).ok_or("sistema não encontrado")?;
                let j = i as i64 + dir as i64;
                let sys = &mut p.graphs[g].systems;
                if j < 0 || j >= sys.len() as i64 {
                    return Err("já está na ponta do grafo".into());
                }
                let j = j as usize;
                if sys[i].phase != sys[j].phase {
                    return Err(format!("a ordem só muda dentro da mesma fase ({}); troque a fase para mover entre fases", sys[i].phase.name()));
                }
                sys.swap(i, j);
                Ok(())
            }),
            Action::Phase(n, dir) => self.edit(|p| {
                let s = sys_mut(p, &n)?;
                let i = Phase::ALL.iter().position(|&x| x == s.phase).unwrap_or(0) as i32;
                s.phase = Phase::ALL[(i + dir).rem_euclid(Phase::ALL.len() as i32) as usize];
                Ok(())
            }),
            Action::Every(n, dir) => self.edit(|p| {
                let s = sys_mut(p, &n)?;
                let i = EVERY_STEPS.iter().position(|&x| x >= s.every).unwrap_or(0) as i32;
                // "+" aumenta a frequência (divide menos), "-" diminui
                let j = (i - dir).clamp(0, EVERY_STEPS.len() as i32 - 1) as usize;
                s.every = EVERY_STEPS[j];
                Ok(())
            }),
            Action::Policy(n, dir) => self.edit(|p| {
                let s = sys_mut(p, &n)?;
                let i = FailPolicy::ALL.iter().position(|&x| x == s.policy).unwrap_or(0) as i32;
                s.policy = FailPolicy::ALL[(i + dir).rem_euclid(FailPolicy::ALL.len() as i32) as usize];
                Ok(())
            }),
            Action::Access(n, t) => {
                let is_static = d.tables.iter().any(|x| x.name == t && x.lifetime == Lifetime::Static);
                self.edit(|p| {
                    let s = sys_mut(p, &n)?;
                    let next = match s.access_kind(&t) {
                        None => Some(false),
                        Some(false) if !is_static => Some(true),
                        _ => None,
                    };
                    s.set_access(&t, next);
                    Ok(())
                });
                if is_static {
                    self.set_status(format!("{} é estática: só pode ser lida", t), INFO);
                }
            }
            Action::OpenCols(n, t) => {
                self.sel = Some(n.clone());
                self.focus = Focus::Cols { system: n, table: t };
            }
            Action::ToggleCol(n, t, c) => {
                let all: Vec<String> = d.tables.iter().find(|x| x.name == t).map(|x| x.columns.iter().map(|c| c.0.clone()).collect()).unwrap_or_default();
                self.edit(|p| {
                    let a = sys_mut(p, &n)?.access_mut(&t).ok_or("o sistema não acessa essa tabela")?;
                    let mut cols = if a.cols.is_empty() { all.clone() } else { a.cols.clone() };
                    if let Some(i) = cols.iter().position(|x| *x == c) {
                        if cols.len() == 1 {
                            return Err("pelo menos uma coluna: para não acessar a tabela, clique na célula até ficar vazia".into());
                        }
                        cols.remove(i);
                    } else {
                        cols.push(c.clone());
                    }
                    // mantém a ordem do catálogo; todas marcadas = "tabela inteira"
                    cols.sort_by_key(|x| all.iter().position(|y| y == x));
                    a.cols = if cols.len() == all.len() { Vec::new() } else { cols };
                    Ok(())
                });
            }
            Action::AllCols(n, t) => self.edit(|p| {
                sys_mut(p, &n)?.access_mut(&t).ok_or("o sistema não acessa essa tabela")?.cols.clear();
                Ok(())
            }),
            Action::EditNote(n) => match std::mem::replace(&mut self.focus, Focus::None) {
                Focus::Note { system, buf } if system == n => self.edit(|p| {
                    sys_mut(p, &n)?.note = buf.trim().to_string();
                    Ok(())
                }),
                _ => {
                    let note = self.project.find(&n).map(|(g, i)| self.project.graphs[g].systems[i].note.clone()).unwrap_or_default();
                    self.sel = Some(n.clone());
                    self.focus = Focus::Note { system: n, buf: note };
                }
            },
            Action::Tab(t) => self.tab = t,
            Action::Save => {
                self.save();
            }
            Action::Undo => match self.undo.pop() {
                Some(p) => {
                    self.redo.push(std::mem::replace(&mut self.project, p));
                    self.set_status("desfeito", INFO);
                }
                None => self.set_status("nada para desfazer", INFO),
            },
            Action::Redo => match self.redo.pop() {
                Some(p) => {
                    self.undo.push(std::mem::replace(&mut self.project, p));
                    self.set_status("refeito", INFO);
                }
                None => self.set_status("nada para refazer", INFO),
            },
            Action::Run => self.launch(false, d),
            Action::Measure => self.launch(true, d),
            Action::Stop => {
                if let Some((child, _)) = &mut self.game {
                    let _ = child.kill();
                }
            }
            Action::Reload => match Project::load(&self.projeto) {
                Ok(p) => {
                    self.undo.push(std::mem::replace(&mut self.project, p.clone()));
                    self.saved = p;
                    self.project_mtime = mtime(&self.projeto);
                    self.set_status("recarregado do disco (Ctrl+Z volta à versão anterior)", OK);
                }
                Err(e) => self.set_status(format!("não consegui recarregar: {}", e.join("; ")), BAD),
            },
            Action::CloseFocus => self.focus = Focus::None,
        }
    }
}

struct Opts {
    projeto: PathBuf,
    shot: Option<PathBuf>,
    tab: Option<Tab>,
    select: Option<String>,
    cols: Option<(String, String)>,
    table: Option<String>,
}

fn parse_opts() -> Opts {
    let mut o = Opts { projeto: PathBuf::from("corrida/projeto.forja"), shot: None, tab: None, select: None, cols: None, table: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--shot" => o.shot = it.next().map(PathBuf::from),
            "--aba" => {
                o.tab = match it.next().as_deref() {
                    Some("grafo") => Some(Tab::Grafo),
                    Some("tabelas") => Some(Tab::Tabelas),
                    _ => Some(Tab::Pipeline),
                }
            }
            "--sel" => o.select = it.next(),
            "--tabela" => o.table = it.next(),
            "--cols" => o.cols = it.next().and_then(|s| s.split_once(':').map(|(a, b)| (a.to_string(), b.to_string()))),
            _ => o.projeto = PathBuf::from(a),
        }
    }
    o
}

/// Um frame direto para PNG, sem janela.
fn shot(mut app: App, o: &Opts, out: &std::path::Path) {
    if let Some(t) = o.tab {
        app.tab = t;
    }
    app.sel = o.select.clone();
    app.sel_table = o.table.clone();
    if let Some((s, t)) = &o.cols {
        app.sel = Some(s.clone());
        app.focus = Focus::Cols { system: s.clone(), table: t.clone() };
    }
    let d = app.derive();
    let mut px = vec![0u32; W * H];
    let mut texts = Vec::new();
    {
        let mouse = forja::platform::Mouse { x: -100, y: -100, ..Default::default() };
        let mut ui = views::Ui::new(Canvas::full(W, H, &mut px), mouse, &mut texts);
        views::draw(&app, &d, &mut ui);
    }
    platform::render_texts_offscreen(W, H, &mut px, &texts);
    if let Err(e) = forja::png::write_png(out, W, H, &px) {
        eprintln!("falha ao gravar {}: {}", out.display(), e);
    }
}

fn main() {
    let opts = parse_opts();
    let projeto = opts.projeto.clone();
    if !projeto.exists() {
        let msg = format!("Não achei {}.\nRode o editor a partir da pasta do projeto (onde fica o Cargo.toml) ou passe o caminho do projeto.forja.", projeto.display());
        eprintln!("{}", msg);
        platform::message_box("forja-editor", &msg);
        std::process::exit(2);
    }
    let mut app = App::new(projeto);
    if let Some(out) = &opts.shot {
        shot(app, &opts, out);
        return;
    }
    let mut win = Window::open("forja-editor", W, H);
    let mut px = vec![0u32; W * H];
    let mut texts: Vec<Text> = Vec::new();
    loop {
        win.pump();
        if win.quit {
            if app.dirty() {
                match platform::ask_yes_no_cancel("forja-editor", "Salvar as alterações em projeto.forja antes de sair?") {
                    Some(true) => {
                        if !app.save() {
                            win.cancel_quit();
                        }
                    }
                    Some(false) => {}
                    None => win.cancel_quit(),
                }
            }
            if win.quit {
                break;
            }
        }
        app.poll();
        let d = app.derive();
        let mut actions = app.handle_keys(&win.keys, &win.chars);
        if win.mouse.wheel != 0 {
            let steps = -win.mouse.wheel / 120;
            if win.mouse.x > views::ISSUES_X && win.mouse.y > views::BOTTOM_Y {
                app.issue_scroll = (app.issue_scroll as i32 + steps * 3).max(0) as usize;
            } else if win.mouse.y < views::BOTTOM_Y {
                app.scroll = (app.scroll + steps * 3).max(0);
            }
        }
        texts.clear();
        {
            let mut ui = views::Ui::new(Canvas::full(W, H, &mut px), win.mouse, &mut texts);
            views::draw(&app, &d, &mut ui);
            actions.append(&mut ui.actions);
        }
        for a in actions {
            app.apply(a, &d);
        }
        let title = format!("forja-editor · {}{}", app.projeto.display(), if app.dirty() { " *" } else { "" });
        win.set_title(&title);
        win.present(&px, &texts);
        std::thread::sleep(Duration::from_millis(12));
    }
}
