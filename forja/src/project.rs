//! O pipeline como dados.
//!
//! - `projeto.forja`: o que você decide. Grafos, sistemas, fases, frequências, leituras e
//!   escritas, políticas de falha. Editado no forja-editor (ou à mão) e lido pelo jogo ao iniciar.
//! - `perfil.forja`: o que o jogo reporta. Catálogo das tabelas como existem no código
//!   (colunas, bytes, linhas), sistemas implementados e medições da última execução.
//!
//! A validação é a mesma nos dois lados: o editor mostra ao vivo os erros que o jogo daria.
//!
//! Formato: seções `[tipo nome]` seguidas de linhas `chave = valor`; `#` comenta a linha.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use crate::analysis::{AccessModel, GraphModel, NodeModel, TableModel};
use crate::schedule::{FailPolicy, Phase, Schedule, SysFn, SystemDecl};
use crate::world::{Layout, Lifetime, World};

// ---------------------------------------------------------------- documento genérico

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub value: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub kind: String,
    pub name: String,
    pub line: usize,
    pub entries: Vec<Entry>,
}

impl Section {
    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }
    fn value(&self, key: &str) -> &str {
        self.get(key).map(|e| e.value.as_str()).unwrap_or("")
    }
}

pub fn parse_doc(text: &str) -> Result<Vec<Section>, Vec<String>> {
    let mut out: Vec<Section> = Vec::new();
    let mut errors = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(inner) = l.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let inner = inner.trim();
            let (kind, name) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
            out.push(Section { kind: kind.to_string(), name: name.trim().to_string(), line, entries: Vec::new() });
            continue;
        }
        match (l.split_once('='), out.last_mut()) {
            (Some((k, v)), Some(sec)) => sec.entries.push(Entry { key: k.trim().to_string(), value: v.trim().to_string(), line }),
            (Some(_), None) => errors.push(format!("linha {}: chave fora de qualquer seção", line)),
            (None, _) => errors.push(format!("linha {}: esperava '[seção]' ou 'chave = valor', achei '{}'", line, l)),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

fn yes(v: &str) -> bool {
    matches!(v, "sim" | "s" | "true" | "1")
}

fn yn(b: bool) -> &'static str {
    if b {
        "sim"
    } else {
        "nao"
    }
}

// ---------------------------------------------------------------- projeto

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessSpec {
    pub table: String,
    /// Vazio = todas as colunas.
    pub cols: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemSpec {
    pub name: String,
    pub phase: Phase,
    pub every: u32,
    pub policy: FailPolicy,
    pub active: bool,
    pub reads: Vec<AccessSpec>,
    pub writes: Vec<AccessSpec>,
    pub note: String,
}

impl SystemSpec {
    /// None = não acessa; Some(false) = lê; Some(true) = escreve.
    pub fn access_kind(&self, table: &str) -> Option<bool> {
        if self.writes.iter().any(|a| a.table == table) {
            Some(true)
        } else if self.reads.iter().any(|a| a.table == table) {
            Some(false)
        } else {
            None
        }
    }

    pub fn access_mut(&mut self, table: &str) -> Option<&mut AccessSpec> {
        self.writes.iter_mut().chain(self.reads.iter_mut()).find(|a| a.table == table)
    }

    pub fn access(&self, table: &str) -> Option<&AccessSpec> {
        self.writes.iter().chain(self.reads.iter()).find(|a| a.table == table)
    }

    /// Troca o tipo de acesso a uma tabela, preservando as colunas quando possível.
    pub fn set_access(&mut self, table: &str, kind: Option<bool>) {
        let cols = self.access(table).map(|a| a.cols.clone()).unwrap_or_default();
        self.reads.retain(|a| a.table != table);
        self.writes.retain(|a| a.table != table);
        let spec = AccessSpec { table: table.to_string(), cols };
        match kind {
            Some(true) => self.writes.push(spec),
            Some(false) => self.reads.push(spec),
            None => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphSpec {
    pub name: String,
    pub render: bool,
    pub systems: Vec<SystemSpec>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Project {
    pub graphs: Vec<GraphSpec>,
}

fn parse_access(v: &str, line: usize) -> Result<Vec<AccessSpec>, String> {
    let mut out = Vec::new();
    for tok in v.split_whitespace() {
        let (table, cols) = match tok.split_once('(') {
            Some((t, rest)) => {
                let inner = rest.strip_suffix(')').ok_or_else(|| format!("linha {}: falta ')' em '{}'", line, tok))?;
                (t, inner.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect())
            }
            None => (tok, Vec::new()),
        };
        if table.is_empty() {
            return Err(format!("linha {}: nome de tabela vazio em '{}'", line, tok));
        }
        out.push(AccessSpec { table: table.to_string(), cols });
    }
    Ok(out)
}

fn fmt_access(v: &[AccessSpec]) -> String {
    v.iter()
        .map(|a| if a.cols.is_empty() { a.table.clone() } else { format!("{}({})", a.table, a.cols.join(",")) })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Project {
    pub fn parse(text: &str) -> Result<Project, Vec<String>> {
        let sections = parse_doc(text)?;
        let mut p = Project::default();
        let mut errors = Vec::new();
        for s in &sections {
            match s.kind.as_str() {
                "grafo" => {
                    for e in &s.entries {
                        if e.key != "render" {
                            errors.push(format!("linha {}: chave desconhecida '{}' em [grafo]", e.line, e.key));
                        }
                    }
                    p.graphs.push(GraphSpec { name: s.name.clone(), render: yes(s.value("render")), systems: Vec::new() });
                }
                "sistema" => {
                    let Some(g) = p.graphs.last_mut() else {
                        errors.push(format!("linha {}: [sistema {}] antes de qualquer [grafo]", s.line, s.name));
                        continue;
                    };
                    let mut spec = SystemSpec {
                        name: s.name.clone(),
                        phase: Phase::Input,
                        every: 1,
                        policy: FailPolicy::Abort,
                        active: true,
                        reads: Vec::new(),
                        writes: Vec::new(),
                        note: String::new(),
                    };
                    let mut has_phase = false;
                    for e in &s.entries {
                        match e.key.as_str() {
                            "fase" => match Phase::from_key(&e.value) {
                                Some(ph) => {
                                    spec.phase = ph;
                                    has_phase = true;
                                }
                                None => {
                                    has_phase = true;
                                    errors.push(format!("linha {}: fase desconhecida '{}'", e.line, e.value));
                                }
                            },
                            "a_cada" => match e.value.parse::<u32>() {
                                Ok(n) => spec.every = n,
                                Err(_) => errors.push(format!("linha {}: a_cada espera um número, achei '{}'", e.line, e.value)),
                            },
                            "falha" => match FailPolicy::from_key(&e.value) {
                                Some(pol) => spec.policy = pol,
                                None => errors.push(format!("linha {}: política desconhecida '{}' (use abortar, pular_tick, desligar, desligar_sem_rollback)", e.line, e.value)),
                            },
                            "ativo" => spec.active = yes(&e.value),
                            "le" => match parse_access(&e.value, e.line) {
                                Ok(v) => spec.reads = v,
                                Err(m) => errors.push(m),
                            },
                            "escreve" => match parse_access(&e.value, e.line) {
                                Ok(v) => spec.writes = v,
                                Err(m) => errors.push(m),
                            },
                            "nota" => spec.note = e.value.clone(),
                            other => errors.push(format!("linha {}: chave desconhecida '{}' em [sistema {}]", e.line, other, s.name)),
                        }
                    }
                    if !has_phase {
                        errors.push(format!("linha {}: [sistema {}] sem 'fase'", s.line, s.name));
                    }
                    g.systems.push(spec);
                }
                other => errors.push(format!("linha {}: seção desconhecida '[{}]'", s.line, other)),
            }
        }
        if errors.is_empty() {
            p.normalize();
            Ok(p)
        } else {
            Err(errors)
        }
    }

    pub fn load(path: &Path) -> Result<Project, Vec<String>> {
        let text = std::fs::read_to_string(path).map_err(|e| vec![format!("não consegui ler {}: {}", path.display(), e)])?;
        Project::parse(&text)
    }

    /// Ordem de execução: fase, depois ordem de declaração (o mesmo que o scheduler faz).
    pub fn normalize(&mut self) {
        for g in &mut self.graphs {
            g.systems.sort_by_key(|s| s.phase);
        }
    }

    pub fn to_text(&self) -> String {
        let mut r = String::new();
        r.push_str("# forja · projeto: o pipeline do jogo como dados.\n");
        r.push_str("# Editado pelo forja-editor ou à mão. O jogo lê este arquivo ao iniciar e recusa\n");
        r.push_str("# declarações inválidas (as mesmas que o editor mostra em vermelho).\n");
        r.push_str("#\n");
        r.push_str("# fase:    Input IA Controle Fisica Colisao Gameplay Direcao VFX Extract Render\n");
        r.push_str("# a_cada:  roda a cada N ticks (60 Hz / N); ignorado no grafo de render\n");
        r.push_str("# falha:   abortar | pular_tick | desligar | desligar_sem_rollback\n");
        r.push_str("# le, escreve: Tabela ou Tabela(coluna,coluna) separadas por espaço\n");
        for g in &self.graphs {
            let _ = write!(r, "\n[grafo {}]\nrender = {}\n", g.name, yn(g.render));
            for s in &g.systems {
                let _ = write!(
                    r,
                    "\n[sistema {}]\nfase = {}\na_cada = {}\nfalha = {}\nativo = {}\nle = {}\nescreve = {}\nnota = {}\n",
                    s.name,
                    s.phase.key(),
                    s.every,
                    s.policy.key(),
                    yn(s.active),
                    fmt_access(&s.reads),
                    fmt_access(&s.writes),
                    s.note
                );
            }
        }
        r.lines().map(str::trim_end).collect::<Vec<_>>().join("
") + "
"
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_text())
    }

    pub fn find(&self, name: &str) -> Option<(usize, usize)> {
        for (g, gs) in self.graphs.iter().enumerate() {
            if let Some(i) = gs.systems.iter().position(|s| s.name == name) {
                return Some((g, i));
            }
        }
        None
    }

    /// Nomes de tabelas citados no projeto (para quando ainda não há catálogo).
    pub fn table_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in self.graphs.iter().flat_map(|g| g.systems.iter()) {
            for a in s.reads.iter().chain(s.writes.iter()) {
                if !out.contains(&a.table) {
                    out.push(a.table.clone());
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------- validação

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub level: Level,
    pub system: Option<String>,
    pub msg: String,
}

/// `tables` = catálogo (vazio se ainda não existe); `implemented` = funções registradas no código
/// (vazio se desconhecido). O jogo aborta com qualquer `Level::Error`.
pub fn validate(p: &Project, tables: &[TableModel], implemented: &[String]) -> Vec<Issue> {
    let mut out = Vec::new();
    let mut push = |level: Level, system: Option<&str>, msg: String| out.push(Issue { level, system: system.map(|s| s.to_string()), msg });
    let mut seen: Vec<&str> = Vec::new();
    for g in &p.graphs {
        for s in &g.systems {
            let sys = Some(s.name.as_str());
            if seen.contains(&s.name.as_str()) {
                push(Level::Error, sys, format!("{}: nome duplicado no projeto", s.name));
            }
            seen.push(&s.name);
            if !s.active {
                push(Level::Info, sys, format!("{}: desativado no pipeline (não roda)", s.name));
                continue;
            }
            if s.every == 0 {
                push(Level::Error, sys, format!("{}: a_cada = 0 não faz sentido", s.name));
            }
            if !implemented.is_empty() && !implemented.contains(&s.name) {
                push(Level::Error, sys, format!("{}: não existe função registrada com esse nome no código", s.name));
            }
            if g.render && s.phase != Phase::Render {
                push(Level::Warning, sys, format!("{}: está no grafo de render mas na fase {}", s.name, s.phase.name()));
            }
            for a in &s.reads {
                if s.writes.iter().any(|w| w.table == a.table) {
                    push(Level::Info, sys, format!("{}: lê e escreve {}; escrever já implica ler", s.name, a.table));
                }
            }
            if tables.is_empty() {
                continue;
            }
            for (a, write) in s.reads.iter().map(|a| (a, false)).chain(s.writes.iter().map(|a| (a, true))) {
                let Some(t) = tables.iter().find(|t| t.name == a.table) else {
                    push(Level::Error, sys, format!("{}: a tabela '{}' não existe no código", s.name, a.table));
                    continue;
                };
                for c in &a.cols {
                    if t.col_bytes(c).is_none() {
                        push(Level::Error, sys, format!("{}: a coluna '{}.{}' não existe", s.name, a.table, c));
                    }
                }
                if write && t.lifetime == Lifetime::Static {
                    push(Level::Error, sys, format!("{}: escreve na tabela estática '{}'", s.name, a.table));
                }
            }
        }
    }
    for name in implemented {
        if !seen.contains(&name.as_str()) {
            push(Level::Warning, None, format!("{}: implementado no código, mas fora do pipeline", name));
        }
    }
    for t in tables {
        let used = p.graphs.iter().flat_map(|g| g.systems.iter()).filter(|s| s.active).any(|s| s.access_kind(&t.name).is_some());
        if !used && !t.external {
            push(Level::Info, None, format!("tabela {} não é usada por nenhum sistema ativo", t.name));
        }
    }
    out.sort_by_key(|i| i.level);
    out
}

// ---------------------------------------------------------------- modelo e grafo a partir do projeto

/// Modelo para análise no editor: só sistemas ativos, tabelas resolvidas pelo nome no catálogo.
pub fn graph_model(g: &GraphSpec, tables: &[TableModel], dur: &HashMap<String, f64>) -> GraphModel {
    let nodes = g
        .systems
        .iter()
        .filter(|s| s.active)
        .map(|s| {
            let mut access: Vec<AccessModel> = Vec::new();
            for (a, write) in s.reads.iter().map(|a| (a, false)).chain(s.writes.iter().map(|a| (a, true))) {
                let Some(t) = tables.iter().position(|t| t.name == a.table) else { continue };
                if let Some(e) = access.iter_mut().find(|e| e.table == t) {
                    e.write |= write;
                    if e.cols.is_empty() || a.cols.is_empty() {
                        e.cols.clear();
                    }
                } else {
                    access.push(AccessModel { table: t, write, cols: a.cols.clone() });
                }
            }
            NodeModel {
                name: s.name.clone(),
                phase: s.phase,
                every: s.every,
                policy: s.policy,
                access,
                dur_ms: dur.get(&s.name).copied().unwrap_or(0.0),
                note: s.note.clone(),
            }
        })
        .collect();
    GraphModel::new(&g.name, g.render, nodes)
}

/// Monta o grafo executável: liga cada sistema do projeto à função de mesmo nome no código.
pub fn build_schedule(g: &GraphSpec, world: &World, fns: &[(&'static str, SysFn)], base_id: u16) -> Result<Schedule, Vec<String>> {
    let mut decls = Vec::new();
    let mut errors = Vec::new();
    for s in g.systems.iter().filter(|s| s.active) {
        let Some(&(name, run)) = fns.iter().find(|(n, _)| *n == s.name) else {
            errors.push(format!("{}: não existe função registrada com esse nome no código", s.name));
            continue;
        };
        let mut d = SystemDecl::new(name, s.phase, run).every(s.every.max(1)).policy(s.policy).note(&s.note);
        for a in &s.reads {
            d = d.access_named(&a.table, &a.cols, false);
        }
        for a in &s.writes {
            d = d.access_named(&a.table, &a.cols, true);
        }
        decls.push(d);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Schedule::build(&g.name, world, decls, base_id, g.render)
}

// ---------------------------------------------------------------- perfil

#[derive(Clone, Debug, Default)]
pub struct SysMeasure {
    pub ms: f64,
    pub runs: u64,
    pub failures: u64,
    /// Tabelas que o sistema de fato acessou na última execução.
    pub touched: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub origin: String,
    pub tables: Vec<TableModel>,
    pub implemented: Vec<String>,
    pub systems: HashMap<String, SysMeasure>,
}

impl Profile {
    pub fn durations(&self) -> HashMap<String, f64> {
        self.systems.iter().map(|(k, v)| (k.clone(), v.ms)).collect()
    }
}

fn lifetime_key(l: Lifetime) -> &'static str {
    match l {
        Lifetime::Static => "estatica",
        Lifetime::Session => "sessao",
        Lifetime::Transient => "transitoria",
    }
}

fn lifetime_from(s: &str) -> Lifetime {
    match s {
        "estatica" => Lifetime::Static,
        "transitoria" => Lifetime::Transient,
        _ => Lifetime::Session,
    }
}

pub fn profile_text(world: &World, scheds: &[&Schedule], implemented: &[&str], origin: &str) -> String {
    let mut r = String::new();
    r.push_str("# forja · perfil: gerado pelo jogo a cada execução. Não editar.\n");
    r.push_str("# Catálogo das tabelas como existem no código + medições da última execução.\n");
    let _ = write!(r, "\n[perfil]\norigem = {}\nimplementados = {}\n", origin, implemented.join(" "));
    for (id, slot) in world.slots.iter().enumerate() {
        let d = &slot.decl;
        let cols: Vec<String> = d.columns.iter().map(|c| format!("{}:{}", c.name, c.bytes)).collect();
        let inv: Vec<&str> = slot.invariant_names().collect();
        let _ = write!(
            r,
            "\n[tabela {}]\nvida = {}\nlayout = {:?}\nlinha = {}\ncolunas = {}\nlinhas = {}\nesparsa = {}\nexterna = {}\ninvariantes = {}\nnota = {}\n",
            d.name,
            lifetime_key(d.lifetime),
            d.layout,
            d.row_size,
            cols.join(" "),
            world.rows(id),
            yn(d.sparse),
            yn(d.external),
            inv.join("; "),
            d.note
        );
    }
    for s in scheds {
        for sys in &s.systems {
            let touched: Vec<&str> = sys
                .access
                .iter()
                .zip(&sys.touched)
                .filter(|(_, t)| t.load(std::sync::atomic::Ordering::Relaxed))
                .map(|(a, _)| world.slots[a.table].decl.name)
                .collect();
            let _ = write!(
                r,
                "\n[medicao {}]\nms = {:.6}\nexecucoes = {}\nfalhas = {}\nacessou = {}\n",
                sys.decl.name,
                sys.avg_ms(),
                sys.runs.load(std::sync::atomic::Ordering::Relaxed),
                sys.failures.load(std::sync::atomic::Ordering::Relaxed),
                touched.join(" ")
            );
        }
    }
    r.lines().map(str::trim_end).collect::<Vec<_>>().join("
") + "
"
}

pub fn write_profile(path: &Path, world: &World, scheds: &[&Schedule], implemented: &[&str], origin: &str) -> std::io::Result<()> {
    std::fs::write(path, profile_text(world, scheds, implemented, origin))
}

pub fn load_profile(path: &Path) -> Result<Profile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("não consegui ler {}: {}", path.display(), e))?;
    let sections = parse_doc(&text).map_err(|e| e.join("; "))?;
    let mut p = Profile::default();
    for s in &sections {
        match s.kind.as_str() {
            "perfil" => {
                p.origin = s.value("origem").to_string();
                p.implemented = s.value("implementados").split_whitespace().map(|x| x.to_string()).collect();
            }
            "tabela" => {
                let columns = s
                    .value("colunas")
                    .split_whitespace()
                    .filter_map(|c| c.split_once(':').and_then(|(n, b)| b.parse().ok().map(|b| (n.to_string(), b))))
                    .collect();
                p.tables.push(TableModel {
                    name: s.name.clone(),
                    lifetime: lifetime_from(s.value("vida")),
                    layout: if s.value("layout") == "SoA" { Layout::SoA } else { Layout::AoS },
                    row_size: s.value("linha").parse().unwrap_or(0),
                    columns,
                    rows: s.value("linhas").parse().unwrap_or(0),
                    sparse: yes(s.value("esparsa")),
                    external: yes(s.value("externa")),
                    note: s.value("nota").to_string(),
                    invariants: s.value("invariantes").split(';').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
                });
            }
            "medicao" => {
                p.systems.insert(
                    s.name.clone(),
                    SysMeasure {
                        ms: s.value("ms").parse().unwrap_or(0.0),
                        runs: s.value("execucoes").parse().unwrap_or(0),
                        failures: s.value("falhas").parse().unwrap_or(0),
                        touched: s.value("acessou").split_whitespace().map(|x| x.to_string()).collect(),
                    },
                );
            }
            _ => {}
        }
    }
    Ok(p)
}

impl Project {
    /// Exporta grafos montados a partir de declarações em código: útil para migrar um
    /// pipeline escrito em Rust para o arquivo de projeto.
    pub fn from_schedules(world: &World, scheds: &[&Schedule]) -> Project {
        let graphs = scheds
            .iter()
            .map(|s| GraphSpec {
                name: s.name.clone(),
                render: s.is_render,
                systems: s
                    .systems
                    .iter()
                    .map(|x| {
                        let spec = |write: bool| -> Vec<AccessSpec> {
                            x.access
                                .iter()
                                .filter(|a| a.write == write)
                                .map(|a| AccessSpec { table: world.slots[a.table].decl.name.to_string(), cols: a.cols.clone() })
                                .collect()
                        };
                        SystemSpec {
                            name: x.decl.name.to_string(),
                            phase: x.decl.phase,
                            every: x.decl.every,
                            policy: x.decl.policy,
                            active: true,
                            reads: spec(false),
                            writes: spec(true),
                            note: x.decl.note.clone(),
                        }
                    })
                    .collect(),
            })
            .collect();
        Project { graphs }
    }
}
