//! Análise sobre as declarações + medições: matriz de acesso, caminho crítico,
//! uso estimado de cache line e sugestões. É o "editor que critica os dados".
//!
//! Tudo trabalha sobre um modelo de dados puro (`TableModel`, `GraphModel`), que pode vir
//! do jogo rodando (World + Schedule) ou dos arquivos do projeto (editor, sem o jogo).

use std::fmt::Write as _;

use crate::schedule::{FailPolicy, Phase, Schedule};
use crate::world::{Layout, Lifetime, World};

// ---------------------------------------------------------------- modelo

#[derive(Clone, Debug)]
pub struct TableModel {
    pub name: String,
    pub lifetime: Lifetime,
    pub layout: Layout,
    pub row_size: usize,
    pub columns: Vec<(String, usize)>,
    pub rows: usize,
    pub sparse: bool,
    pub external: bool,
    pub note: String,
    pub invariants: Vec<String>,
}

impl TableModel {
    pub fn used_bytes(&self) -> usize {
        self.columns.iter().map(|c| c.1).sum()
    }
    pub fn row_bytes(&self) -> usize {
        match self.layout {
            Layout::SoA => self.used_bytes(),
            Layout::AoS => self.row_size.max(self.used_bytes()),
        }
    }
    pub fn col_bytes(&self, name: &str) -> Option<usize> {
        self.columns.iter().find(|c| c.0 == name).map(|c| c.1)
    }
    pub fn total_bytes(&self) -> usize {
        self.rows * self.row_bytes()
    }
    pub fn padding(&self) -> usize {
        match self.layout {
            Layout::AoS => self.row_size.saturating_sub(self.used_bytes()),
            Layout::SoA => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessModel {
    pub table: usize,
    pub write: bool,
    /// Vazio = todas as colunas.
    pub cols: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NodeModel {
    pub name: String,
    pub phase: Phase,
    pub every: u32,
    pub policy: FailPolicy,
    pub access: Vec<AccessModel>,
    pub dur_ms: f64,
    pub note: String,
}

impl NodeModel {
    pub fn access_to(&self, t: usize) -> Option<&AccessModel> {
        self.access.iter().find(|a| a.table == t)
    }
    pub fn writes(&self, t: usize) -> bool {
        self.access_to(t).is_some_and(|a| a.write)
    }
    pub fn reads_only(&self, t: usize) -> bool {
        self.access_to(t).is_some_and(|a| !a.write)
    }
}

pub fn conflicts(a: &NodeModel, b: &NodeModel) -> bool {
    a.access.iter().any(|x| b.access.iter().any(|y| x.table == y.table && (x.write || y.write)))
}

#[derive(Clone, Debug)]
pub struct GraphModel {
    pub name: String,
    pub render: bool,
    pub nodes: Vec<NodeModel>,
    pub deps: Vec<Vec<usize>>,
    pub dependents: Vec<Vec<usize>>,
}

impl GraphModel {
    /// `nodes` já em ordem de execução (fase, depois ordem de declaração).
    pub fn new(name: &str, render: bool, nodes: Vec<NodeModel>) -> GraphModel {
        let n = nodes.len();
        let mut deps = vec![Vec::new(); n];
        let mut dependents = vec![Vec::new(); n];
        for j in 0..n {
            for i in 0..j {
                if conflicts(&nodes[i], &nodes[j]) {
                    deps[j].push(i);
                    dependents[i].push(j);
                }
            }
        }
        GraphModel { name: name.to_string(), render, nodes, deps, dependents }
    }

    pub fn durations(&self) -> Vec<f64> {
        self.nodes.iter().map(|n| n.dur_ms).collect()
    }

    pub fn critical_path(&self) -> CritPath {
        critical_path_dag(&self.deps, &self.dependents, &self.durations())
    }

    pub fn levels(&self) -> Vec<usize> {
        levels_dag(&self.deps)
    }

    /// Tabelas que forçam a ordem entre i e j.
    pub fn conflict_tables(&self, i: usize, j: usize, tables: &[TableModel]) -> Vec<String> {
        let (a, b) = (&self.nodes[i], &self.nodes[j]);
        a.access
            .iter()
            .filter(|x| b.access.iter().any(|y| y.table == x.table && (x.write || y.write)))
            .map(|x| tables.get(x.table).map(|t| t.name.clone()).unwrap_or_default())
            .collect()
    }

    /// Bytes copiados por execução para permitir rollback.
    pub fn txn_bytes(&self, i: usize, tables: &[TableModel]) -> usize {
        let n = &self.nodes[i];
        if !n.policy.transactional() {
            return 0;
        }
        n.access.iter().filter(|a| a.write).filter_map(|a| tables.get(a.table)).map(|t| t.total_bytes()).sum()
    }

    /// Arestas sem as redundantes (i -> k -> j torna i -> j desnecessária): para desenhar.
    pub fn reduced_edges(&self) -> Vec<(usize, usize)> {
        let n = self.nodes.len();
        let mut reach = vec![vec![false; n]; n];
        for j in 0..n {
            for &d in &self.deps[j] {
                reach[d][j] = true;
            }
        }
        for k in 0..n {
            for i in 0..n {
                if reach[i][k] {
                    for j in 0..n {
                        if reach[k][j] {
                            reach[i][j] = true;
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        for j in 0..n {
            for &i in &self.deps[j] {
                let redundant = self.deps[j].iter().any(|&k| k != i && reach[i][k]);
                if !redundant {
                    out.push((i, j));
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------- grafo

pub struct CritPath {
    pub es: Vec<f64>,
    pub ef: Vec<f64>,
    pub slack: Vec<f64>,
    pub path: Vec<usize>,
    /// Duração do caminho crítico (span).
    pub span: f64,
    /// Soma de todas as durações (work). work / span = paralelismo máximo teórico.
    pub work: f64,
}

impl CritPath {
    pub fn on_path(&self, i: usize) -> bool {
        self.path.contains(&i)
    }
    pub fn parallelism(&self) -> f64 {
        if self.span > 0.0 {
            self.work / self.span
        } else {
            1.0
        }
    }
}

/// Os nós estão em ordem topológica (deps só apontam para índices menores).
pub fn critical_path_dag(deps: &[Vec<usize>], dependents: &[Vec<usize>], dur: &[f64]) -> CritPath {
    let n = deps.len();
    let mut es = vec![0.0; n];
    let mut ef = vec![0.0; n];
    for j in 0..n {
        es[j] = deps[j].iter().map(|&d| ef[d]).fold(0.0, f64::max);
        ef[j] = es[j] + dur[j];
    }
    let span = ef.iter().copied().fold(0.0, f64::max);
    let mut lf = vec![span; n];
    for i in (0..n).rev() {
        for &d in &dependents[i] {
            lf[i] = lf[i].min(lf[d] - dur[d]);
        }
    }
    let slack: Vec<f64> = (0..n).map(|i| (lf[i] - dur[i] - es[i]).max(0.0)).collect();
    let mut path = Vec::new();
    if let Some(mut cur) = (0..n).max_by(|&a, &b| ef[a].total_cmp(&ef[b])) {
        loop {
            path.push(cur);
            let next = deps[cur].iter().copied().filter(|&d| (ef[d] - es[cur]).abs() < 1e-9).max_by(|&a, &b| ef[a].total_cmp(&ef[b]));
            match next {
                Some(d) if es[cur] > 0.0 => cur = d,
                _ => break,
            }
        }
    }
    path.reverse();
    CritPath { es, ef, slack, path, span, work: dur.iter().sum() }
}

/// Profundidade de cada nó no DAG: nós no mesmo nível podem rodar juntos.
pub fn levels_dag(deps: &[Vec<usize>]) -> Vec<usize> {
    let mut lv = vec![0; deps.len()];
    for j in 0..deps.len() {
        lv[j] = deps[j].iter().map(|&d| lv[d] + 1).max().unwrap_or(0);
    }
    lv
}

pub struct AccessStat {
    pub useful: usize,
    pub loaded: usize,
}

impl AccessStat {
    pub fn util(&self) -> f64 {
        if self.loaded == 0 {
            1.0
        } else {
            self.useful as f64 / self.loaded as f64
        }
    }
}

/// Bytes úteis vs carregados quando um sistema varre a tabela com as colunas declaradas.
/// SoA: só as colunas tocadas vêm para o cache. AoS: a linha inteira vem junto.
pub fn access_stat_model(t: &TableModel, cols: &[String]) -> Option<AccessStat> {
    if t.sparse {
        return None;
    }
    let used: usize = if cols.is_empty() { t.used_bytes() } else { cols.iter().filter_map(|c| t.col_bytes(c)).sum() };
    let loaded_row = match t.layout {
        Layout::SoA => used,
        Layout::AoS => t.row_bytes(),
    };
    Some(AccessStat { useful: t.rows * used, loaded: t.rows * loaded_row })
}

pub fn fmt_bytes(b: usize) -> String {
    if b >= 1 << 20 {
        format!("{:.1} MB", b as f64 / (1 << 20) as f64)
    } else if b >= 1 << 10 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{} B", b)
    }
}

// ---------------------------------------------------------------- adaptadores do runtime

pub fn table_model(world: &World, id: usize) -> TableModel {
    let s = &world.slots[id];
    let d = &s.decl;
    TableModel {
        name: d.name.to_string(),
        lifetime: d.lifetime,
        layout: d.layout,
        row_size: d.row_size,
        columns: d.columns.iter().map(|c| (c.name.to_string(), c.bytes)).collect(),
        rows: world.rows(id),
        sparse: d.sparse,
        external: d.external,
        note: d.note.to_string(),
        invariants: s.invariant_names().map(|n| n.to_string()).collect(),
    }
}

pub fn table_models(world: &World) -> Vec<TableModel> {
    (0..world.slots.len()).map(|i| table_model(world, i)).collect()
}

pub fn graph_model(s: &Schedule, dur: &[f64]) -> GraphModel {
    let nodes = s
        .systems
        .iter()
        .enumerate()
        .map(|(i, x)| NodeModel {
            name: x.decl.name.to_string(),
            phase: x.decl.phase,
            every: x.decl.every,
            policy: x.decl.policy,
            access: x.access.iter().map(|a| AccessModel { table: a.table, write: a.write, cols: a.cols.clone() }).collect(),
            dur_ms: dur.get(i).copied().unwrap_or(0.0),
            note: x.decl.note.clone(),
        })
        .collect();
    GraphModel::new(&s.name, s.is_render, nodes)
}

pub fn avg_durations(s: &Schedule) -> Vec<f64> {
    s.systems.iter().map(|x| x.avg_ms()).collect()
}

pub fn critical_path(s: &Schedule, dur: &[f64]) -> CritPath {
    let deps: Vec<Vec<usize>> = s.systems.iter().map(|x| x.deps.clone()).collect();
    let dependents: Vec<Vec<usize>> = s.systems.iter().map(|x| x.dependents.clone()).collect();
    critical_path_dag(&deps, &dependents, dur)
}

pub fn levels(s: &Schedule) -> Vec<usize> {
    levels_dag(&s.systems.iter().map(|x| x.deps.clone()).collect::<Vec<_>>())
}

pub fn access_stat(world: &World, table: usize, cols: &[String]) -> Option<AccessStat> {
    access_stat_model(&table_model(world, table), cols)
}

pub fn insights(world: &World, scheds: &[&Schedule], durs: &[Vec<f64>]) -> Vec<String> {
    let tables = table_models(world);
    let graphs: Vec<GraphModel> = scheds.iter().zip(durs).map(|(s, d)| graph_model(s, d)).collect();
    insights_model(&tables, &graphs)
}

pub fn report_markdown(world: &World, scheds: &[&Schedule], durs: &[Vec<f64>], header: &str) -> String {
    let tables = table_models(world);
    let graphs: Vec<GraphModel> = scheds.iter().zip(durs).map(|(s, d)| graph_model(s, d)).collect();
    report_markdown_model(&tables, &graphs, header)
}

// ---------------------------------------------------------------- sugestões

/// Gera as sugestões a partir das declarações e dos tempos (em ms) de cada grafo.
pub fn insights_model(tables: &[TableModel], graphs: &[GraphModel]) -> Vec<String> {
    let mut out = Vec::new();
    let tname = |t: usize| tables.get(t).map(|x| x.name.as_str()).unwrap_or("?");

    for g in graphs {
        let cp = g.critical_path();
        if cp.span > 0.0 && !cp.path.is_empty() {
            let names: Vec<&str> = cp.path.iter().map(|&i| g.nodes[i].name.as_str()).collect();
            let top = cp.path.iter().copied().max_by(|&a, &b| g.nodes[a].dur_ms.total_cmp(&g.nodes[b].dur_ms)).unwrap();
            out.push(format!(
                "[{}] caminho crítico {:.2} ms: {}. {} é {:.0}% dele.",
                g.name,
                cp.span,
                names.join(" -> "),
                g.nodes[top].name,
                100.0 * g.nodes[top].dur_ms / cp.span
            ));
            out.push(format!(
                "[{}] trabalho {:.2} ms / caminho {:.2} ms = paralelismo máximo {:.2}x com as declarações atuais.",
                g.name,
                cp.work,
                cp.span,
                cp.parallelism()
            ));
            for (i, n) in g.nodes.iter().enumerate() {
                if cp.slack[i] > 0.15 && n.dur_ms > 0.02 {
                    out.push(format!("[{}] {} tem {:.2} ms de folga: otimizá-lo não reduz o tempo do grafo.", g.name, n.name, cp.slack[i]));
                }
            }
        }
        for t in 0..tables.len() {
            let writers: Vec<&str> = g.nodes.iter().filter(|n| n.writes(t)).map(|n| n.name.as_str()).collect();
            if writers.len() >= 3 {
                out.push(format!(
                    "[{}] {} tem {} escritores ({}): eles nunca rodam em paralelo. Separar por produtor libera paralelismo.",
                    g.name,
                    tname(t),
                    writers.len(),
                    writers.join(", ")
                ));
            }
        }
        for i in 0..g.nodes.len() {
            let b = g.txn_bytes(i, tables);
            if b > 32 * 1024 {
                out.push(format!(
                    "[{}] {} copia {} por execução para poder fazer rollback ({}).",
                    g.name,
                    g.nodes[i].name,
                    fmt_bytes(b),
                    g.nodes[i].policy.name()
                ));
            }
        }
    }

    for g in graphs {
        for n in &g.nodes {
            for a in &n.access {
                let Some(t) = tables.get(a.table) else { continue };
                if t.layout != Layout::AoS || t.rows < 2 {
                    continue;
                }
                if let Some(st) = access_stat_model(t, &a.cols) {
                    if st.util() < 0.5 {
                        let cols = if a.cols.is_empty() { "todas".to_string() } else { a.cols.join(", ") };
                        out.push(format!(
                            "{} usa {:.0}% de cada linha de {} ({} de {} B; colunas: {}). Separe as colunas quentes numa tabela própria.",
                            n.name,
                            st.util() * 100.0,
                            t.name,
                            st.useful / t.rows.max(1),
                            t.row_bytes(),
                            cols
                        ));
                    }
                }
            }
        }
    }
    for (ti, t) in tables.iter().enumerate() {
        let pad = t.padding();
        if t.layout == Layout::AoS && t.row_size > 0 && pad * 10 > t.row_size && t.rows > 1 {
            out.push(format!("{}: {} B de padding por linha ({:.0}% da linha). Reordenar campos reduz o tamanho.", t.name, pad, 100.0 * pad as f64 / t.row_size as f64));
        }
        let readers = graphs.iter().flat_map(|g| g.nodes.iter()).filter(|n| n.reads_only(ti)).count();
        let writers = graphs.iter().flat_map(|g| g.nodes.iter()).filter(|n| n.writes(ti)).count();
        if writers == 1 && readers == 0 && !t.external {
            out.push(format!("{} é escrita mas nunca lida por outro sistema: recurso não consumido?", t.name));
        }
        if t.lifetime == Lifetime::Static && readers >= 3 {
            out.push(format!("{} é estática e lida por {} sistemas: nunca bloqueia ninguém.", t.name, readers));
        }
    }
    out
}

/// Relatório em Markdown com tudo que o editor mostra (usado pelo `--analyze`).
pub fn report_markdown_model(tables: &[TableModel], graphs: &[GraphModel], header: &str) -> String {
    let mut r = String::new();
    let _ = writeln!(r, "# Análise do projeto (forja)\n\n{}\n", header);

    let _ = writeln!(r, "## Tabelas\n");
    let _ = writeln!(r, "| Tabela | Vida | Layout | Linhas | B/linha | Padding | Total | Nota |");
    let _ = writeln!(r, "|---|---|---|---:|---:|---:|---:|---|");
    for t in tables {
        let _ = writeln!(
            r,
            "| {} | {} | {:?} | {} | {} | {} | {} | {} |",
            t.name,
            t.lifetime.name(),
            t.layout,
            t.rows,
            t.row_bytes(),
            t.padding(),
            fmt_bytes(t.total_bytes()),
            t.note
        );
    }

    for g in graphs {
        let cp = g.critical_path();
        let lv = g.levels();
        let _ = writeln!(r, "\n## Grafo `{}`\n", g.name);
        let _ = writeln!(r, "Caminho crítico: **{:.3} ms**; trabalho total {:.3} ms; paralelismo máximo **{:.2}x**.\n", cp.span, cp.work, cp.parallelism());
        let _ = writeln!(r, "| Nível | Sistema | Fase | Freq. | Falha | ms (média) | Folga ms | Crítico | Depende de |");
        let _ = writeln!(r, "|---:|---|---|---|---|---:|---:|:-:|---|");
        for (i, n) in g.nodes.iter().enumerate() {
            let deps: Vec<&str> = g.deps[i].iter().map(|&d| g.nodes[d].name.as_str()).collect();
            let freq = if g.render { "frame".to_string() } else { format!("{} Hz", 60 / n.every.max(1)) };
            let _ = writeln!(
                r,
                "| {} | {} | {} | {} | {} | {:.3} | {:.3} | {} | {} |",
                lv[i],
                n.name,
                n.phase.name(),
                freq,
                n.policy.name(),
                n.dur_ms,
                cp.slack[i],
                if cp.on_path(i) { "●" } else { "" },
                deps.join(", ")
            );
        }
        let _ = writeln!(r, "\n### Matriz de acesso (R = lê, W = escreve)\n");
        let used: Vec<usize> = (0..tables.len()).filter(|&t| g.nodes.iter().any(|n| n.access_to(t).is_some())).collect();
        let _ = write!(r, "| Sistema |");
        for &t in &used {
            let _ = write!(r, " {} |", tables[t].name);
        }
        let _ = write!(r, "\n|---|");
        for _ in &used {
            let _ = write!(r, ":-:|");
        }
        let _ = writeln!(r);
        for n in &g.nodes {
            let _ = write!(r, "| {} |", n.name);
            for &t in &used {
                let cell = match n.access_to(t) {
                    Some(a) if a.write => "W",
                    Some(_) => "R",
                    None => "",
                };
                let _ = write!(r, " {} |", cell);
            }
            let _ = writeln!(r);
        }
    }

    let _ = writeln!(r, "\n## Uso de cache por acesso\n");
    let _ = writeln!(r, "| Sistema | Tabela | Colunas | Úteis | Carregados | Uso |");
    let _ = writeln!(r, "|---|---|---|---:|---:|---:|");
    for g in graphs {
        for n in &g.nodes {
            for a in &n.access {
                let Some(t) = tables.get(a.table) else { continue };
                if let Some(st) = access_stat_model(t, &a.cols) {
                    let cols = if a.cols.is_empty() { "todas".into() } else { a.cols.join(", ") };
                    let _ = writeln!(r, "| {} | {} | {} | {} | {} | {:.0}% |", n.name, t.name, cols, fmt_bytes(st.useful), fmt_bytes(st.loaded), st.util() * 100.0);
                }
            }
        }
    }

    let _ = writeln!(r, "\n## Sugestões\n");
    for i in insights_model(tables, graphs) {
        let _ = writeln!(r, "- {}", i);
    }
    r
}
