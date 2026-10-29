//! Sistemas declarados e o grafo que os executa.
//!
//! Cada sistema é escrito como uma função monolítica, mas declara o que lê e escreve,
//! em que fase roda, a frequência e a política de falha. A partir disso:
//! - o grafo de dependências sai das declarações (conflito de leitura/escrita = aresta);
//! - tudo que não conflita roda em paralelo no job pool;
//! - acesso não declarado vira erro na hora (o "borrow checker" do frame);
//! - sistemas com política recuperável têm escrita transacional (rollback em falha).

use std::any::TypeId;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{RwLockReadGuard, RwLockWriteGuard, TryLockError};

use crate::crash;
use crate::draw::Canvas;
use crate::jobs::{self, JobPool, Scope};
use crate::profiler::{self, now_ns, Zone, NO_SUB, RENDER_TICK};
use crate::world::{Erased, Lifetime, Table, World};

pub const SUB_TXN: u16 = u16::MAX - 1;
pub const SUB_INV: u16 = u16::MAX - 2;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Phase {
    Input,
    AI,
    Control,
    Physics,
    Collision,
    Gameplay,
    Direction,
    VFX,
    Extract,
    Render,
}

impl Phase {
    pub const ALL: [Phase; 10] = [
        Phase::Input,
        Phase::AI,
        Phase::Control,
        Phase::Physics,
        Phase::Collision,
        Phase::Gameplay,
        Phase::Direction,
        Phase::VFX,
        Phase::Extract,
        Phase::Render,
    ];

    /// Nome sem acento usado no arquivo de projeto.
    pub fn key(self) -> &'static str {
        match self {
            Phase::Input => "Input",
            Phase::AI => "IA",
            Phase::Control => "Controle",
            Phase::Physics => "Fisica",
            Phase::Collision => "Colisao",
            Phase::Gameplay => "Gameplay",
            Phase::Direction => "Direcao",
            Phase::VFX => "VFX",
            Phase::Extract => "Extract",
            Phase::Render => "Render",
        }
    }

    pub fn from_key(s: &str) -> Option<Phase> {
        Phase::ALL.into_iter().find(|p| p.key().eq_ignore_ascii_case(s) || p.name().eq_ignore_ascii_case(s))
    }

    pub fn name(self) -> &'static str {
        match self {
            Phase::Input => "Input",
            Phase::AI => "IA",
            Phase::Control => "Controle",
            Phase::Physics => "Física",
            Phase::Collision => "Colisão",
            Phase::Gameplay => "Gameplay",
            Phase::Direction => "Direção",
            Phase::VFX => "VFX",
            Phase::Extract => "Extract",
            Phase::Render => "Render",
        }
    }

    pub fn color(self) -> u32 {
        match self {
            Phase::Input => 0x8A8F98,
            Phase::AI => 0xB07CFF,
            Phase::Control => 0x7C9CFF,
            Phase::Physics => 0x3FB8FF,
            Phase::Collision => 0x2DD4BF,
            Phase::Gameplay => 0x84CC16,
            Phase::Direction => 0xFACC15,
            Phase::VFX => 0xFF7AC6,
            Phase::Extract => 0xA3A3A3,
            Phase::Render => 0xFB923C,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FailPolicy {
    /// Relatório completo + minidump e encerra.
    Abort,
    /// Desfaz as escritas do tick e segue; o sistema roda de novo no próximo tick.
    SkipTick,
    /// Desfaz as escritas e desliga o sistema até alguém religar (tecla R no editor).
    Disable,
    /// Desliga sem rollback: para saídas regeneradas a cada frame (framebuffer),
    /// onde copiar a tabela para poder desfazer custaria mais que o próprio trabalho.
    DisableNoRollback,
}

impl FailPolicy {
    pub const ALL: [FailPolicy; 4] = [FailPolicy::Abort, FailPolicy::SkipTick, FailPolicy::Disable, FailPolicy::DisableNoRollback];

    /// Nome usado no arquivo de projeto.
    pub fn key(self) -> &'static str {
        match self {
            FailPolicy::Abort => "abortar",
            FailPolicy::SkipTick => "pular_tick",
            FailPolicy::Disable => "desligar",
            FailPolicy::DisableNoRollback => "desligar_sem_rollback",
        }
    }

    pub fn from_key(s: &str) -> Option<FailPolicy> {
        FailPolicy::ALL.into_iter().find(|p| p.key() == s)
    }

    pub fn name(self) -> &'static str {
        match self {
            FailPolicy::Abort => "abortar",
            FailPolicy::SkipTick => "rollback + pular tick",
            FailPolicy::Disable => "rollback + desligar",
            FailPolicy::DisableNoRollback => "desligar (sem rollback)",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            FailPolicy::Abort => "abort",
            FailPolicy::SkipTick => "skip",
            FailPolicy::Disable => "off",
            FailPolicy::DisableNoRollback => "off*",
        }
    }
    pub fn transactional(self) -> bool {
        matches!(self, FailPolicy::SkipTick | FailPolicy::Disable)
    }
}

#[derive(Clone, Debug)]
pub struct Access {
    /// Tipo Rust da tabela (declaração em código) ou None (declaração por nome, vinda do projeto).
    pub ty: Option<TypeId>,
    pub table: String,
    pub cols: Vec<String>,
    pub write: bool,
}

pub type SysFn = fn(&mut SysCtx);

pub struct SystemDecl {
    pub name: &'static str,
    pub phase: Phase,
    pub run: SysFn,
    pub access: Vec<Access>,
    pub every: u32,
    pub policy: FailPolicy,
    pub note: String,
}

impl SystemDecl {
    pub fn new(name: &'static str, phase: Phase, run: SysFn) -> SystemDecl {
        SystemDecl { name, phase, run, access: Vec::new(), every: 1, policy: FailPolicy::Abort, note: String::new() }
    }

    fn push<T: 'static>(mut self, cols: &[&'static str], write: bool) -> SystemDecl {
        let cols = cols.iter().map(|c| c.to_string()).collect();
        self.access.push(Access { ty: Some(TypeId::of::<T>()), table: std::any::type_name::<T>().to_string(), cols, write });
        self
    }

    /// Acesso pelo nome da tabela (usado ao montar o grafo a partir do arquivo de projeto).
    pub fn access_named(mut self, table: &str, cols: &[String], write: bool) -> SystemDecl {
        self.access.push(Access { ty: None, table: table.to_string(), cols: cols.to_vec(), write });
        self
    }

    /// Lê a tabela inteira.
    pub fn reads<T: 'static>(self) -> SystemDecl {
        self.push::<T>(&[], false)
    }
    /// Lê só estas colunas (alimenta a análise de cache).
    pub fn reads_cols<T: 'static>(self, cols: &[&'static str]) -> SystemDecl {
        self.push::<T>(cols, false)
    }
    pub fn writes<T: 'static>(self) -> SystemDecl {
        self.push::<T>(&[], true)
    }
    pub fn writes_cols<T: 'static>(self, cols: &[&'static str]) -> SystemDecl {
        self.push::<T>(cols, true)
    }
    /// Roda a cada `n` ticks (60 Hz / n).
    pub fn every(mut self, n: u32) -> SystemDecl {
        self.every = n;
        self
    }
    pub fn policy(mut self, p: FailPolicy) -> SystemDecl {
        self.policy = p;
        self
    }
    pub fn note(mut self, s: &str) -> SystemDecl {
        self.note = s.to_string();
        self
    }
}

#[derive(Clone, Debug)]
pub struct TableAccess {
    pub table: usize,
    pub write: bool,
    /// Vazio = todas as colunas.
    pub cols: Vec<String>,
}

pub struct SysRt {
    pub decl: SystemDecl,
    pub id: u16,
    pub access: Vec<TableAccess>,
    pub reads: Vec<usize>,
    pub writes: Vec<usize>,
    pub deps: Vec<usize>,
    pub dependents: Vec<usize>,
    pub enabled: AtomicBool,
    pub last_ns: AtomicU64,
    avg_ns: AtomicU64,
    pub runs: AtomicU64,
    pub failures: AtomicU64,
    pub txn_bytes: AtomicU64,
    /// Quais acessos declarados foram de fato usados (paralelo a `access`).
    pub touched: Vec<AtomicBool>,
}

impl SysRt {
    pub fn avg_ms(&self) -> f64 {
        f64::from_bits(self.avg_ns.load(Ordering::Relaxed)) / 1e6
    }

    pub fn access_to(&self, table: usize) -> Option<&TableAccess> {
        self.access.iter().find(|a| a.table == table)
    }

    pub fn conflicts(&self, o: &SysRt) -> bool {
        self.access.iter().any(|a| o.access.iter().any(|b| a.table == b.table && (a.write || b.write)))
    }
}

pub struct Schedule {
    pub name: String,
    pub systems: Vec<SysRt>,
    pub base_id: u16,
    pub is_render: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TickInfo {
    pub tick: u64,
    pub dt: f32,
    /// Índice do tick dentro do frame (0..), para o profiler.
    pub sub: u8,
    /// Fração entre o último tick e o próximo (interpolação no render).
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct RunOpts {
    pub invariants: bool,
}

impl Schedule {
    /// Resolve e valida as declarações. Os erros são o que um editor mostraria
    /// antes de deixar rodar: tabela inexistente, coluna inexistente, escrita em estático.
    pub fn build(name: &str, world: &World, mut decls: Vec<SystemDecl>, base_id: u16, is_render: bool) -> Result<Schedule, Vec<String>> {
        decls.sort_by_key(|d| d.phase);
        let mut errors = Vec::new();
        let mut systems: Vec<SysRt> = Vec::new();
        for (i, d) in decls.into_iter().enumerate() {
            if systems.iter().any(|s| s.decl.name == d.name) {
                errors.push(format!("sistema duplicado: {}", d.name));
            }
            if d.every == 0 {
                errors.push(format!("{}: every(0) não faz sentido", d.name));
            }
            let mut acc: Vec<TableAccess> = Vec::new();
            for a in &d.access {
                let found = match a.ty {
                    Some(ty) => world.try_id(ty),
                    None => world.id_by_name(&a.table),
                };
                let Some(t) = found else {
                    errors.push(format!("{}: declara a tabela '{}', que não está registrada", d.name, a.table));
                    continue;
                };
                let td = &world.slots[t].decl;
                for c in &a.cols {
                    if td.col_bytes(c).is_none() {
                        errors.push(format!("{}: coluna '{}.{}' não existe", d.name, td.name, c));
                    }
                }
                if a.write && td.lifetime == Lifetime::Static {
                    errors.push(format!("{}: escreve na tabela estática '{}'", d.name, td.name));
                }
                if let Some(e) = acc.iter_mut().find(|e| e.table == t) {
                    e.write |= a.write;
                    if e.cols.is_empty() || a.cols.is_empty() {
                        e.cols.clear();
                    } else {
                        for c in &a.cols {
                            if !e.cols.contains(c) {
                                e.cols.push(c.clone());
                            }
                        }
                    }
                } else {
                    acc.push(TableAccess { table: t, write: a.write, cols: a.cols.clone() });
                }
            }
            let touched = acc.iter().map(|_| AtomicBool::new(false)).collect();
            let reads = acc.iter().map(|a| a.table).collect();
            let writes = acc.iter().filter(|a| a.write).map(|a| a.table).collect();
            systems.push(SysRt {
                decl: d,
                id: base_id + i as u16,
                access: acc,
                reads,
                writes,
                deps: Vec::new(),
                dependents: Vec::new(),
                enabled: AtomicBool::new(true),
                last_ns: AtomicU64::new(0),
                avg_ns: AtomicU64::new(0f64.to_bits()),
                runs: AtomicU64::new(0),
                failures: AtomicU64::new(0),
                txn_bytes: AtomicU64::new(0),
                touched,
            });
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        // Aresta i -> j quando i vem antes (fase/ordem de declaração) e os dois conflitam.
        for j in 0..systems.len() {
            for i in 0..j {
                if systems[i].conflicts(&systems[j]) {
                    systems[j].deps.push(i);
                    systems[i].dependents.push(j);
                }
            }
        }
        let names: Vec<(&'static str, &'static str)> = systems.iter().map(|s| (s.decl.name, s.decl.phase.name())).collect();
        crash::register_systems(base_id, &names);
        Ok(Schedule { name: name.to_string(), systems, base_id, is_render })
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.systems.iter().position(|s| s.decl.name == name)
    }

    pub fn enable_all(&self) -> usize {
        let mut n = 0;
        for s in &self.systems {
            if !s.enabled.swap(true, Ordering::Relaxed) {
                crash::set_disabled(s.id, false);
                crash::breadcrumb(format!("{} religado pelo editor", s.decl.name));
                n += 1;
            }
        }
        n
    }

    pub fn run(&self, world: &World, pool: &JobPool, info: TickInfo, opts: RunOpts) {
        let sh = RunShared {
            sched: self,
            world,
            pool,
            info,
            opts,
            remaining: self.systems.iter().map(|s| AtomicUsize::new(s.deps.len())).collect(),
        };
        let sh = &sh;
        pool.scope(|s| {
            for (i, sys) in self.systems.iter().enumerate() {
                if sys.deps.is_empty() {
                    s.spawn(move |s| run_node(s, sh, i));
                }
            }
        });
    }

    fn exec(&self, i: usize, sh: &RunShared) {
        let rt = &self.systems[i];
        if !rt.enabled.load(Ordering::Relaxed) {
            return;
        }
        if !self.is_render && sh.info.tick % rt.decl.every as u64 != 0 {
            return;
        }
        let lane = jobs::lane();
        let prev = crash::enter_system(lane, rt.id);
        let _ = crash::take_panic_text(); // descarta resto de pânico antigo desta thread
        let prof = profiler::get();
        let tick_tag = if self.is_render { RENDER_TICK } else { sh.info.sub };

        let t0 = now_ns();
        let transactional = rt.decl.policy.transactional();
        let snapshots: Vec<(usize, Erased)> = if transactional { rt.writes.iter().map(|&t| (t, sh.world.snapshot(t))).collect() } else { Vec::new() };
        let t1 = now_ns();

        let mut ctx = SysCtx { world: sh.world, sys: rt, info: sh.info, pool: sh.pool, tick_tag };
        let result = catch_unwind(AssertUnwindSafe(|| (rt.decl.run)(&mut ctx)));
        let t2 = now_ns();

        let mut failure = match result {
            Ok(()) => None,
            Err(p) => Some(crash::take_panic_text().unwrap_or_else(|| crash::payload_str(&*p))),
        };
        let checks = sh.opts.invariants && rt.writes.iter().any(|&t| sh.world.has_invariants(t));
        if failure.is_none() && checks {
            for &t in &rt.writes {
                if let Err(e) = sh.world.check(t) {
                    failure = Some(format!("invariante violada na tabela '{}' depois de '{}': {}", sh.world.slots[t].decl.name, rt.decl.name, e));
                    break;
                }
            }
        }
        let t3 = now_ns();

        let l = lane as u8;
        prof.zone(Zone { name: rt.decl.name, sys: rt.id, sub: NO_SUB, lane: l, tick: tick_tag, start: t0, end: t3 });
        if transactional {
            prof.zone(Zone { name: "txn", sys: rt.id, sub: SUB_TXN, lane: l, tick: tick_tag, start: t0, end: t1 });
            let bytes: usize = rt.writes.iter().map(|&t| sh.world.total_bytes(t)).sum();
            rt.txn_bytes.store(bytes as u64, Ordering::Relaxed);
        }
        if checks {
            prof.zone(Zone { name: "invariantes", sys: rt.id, sub: SUB_INV, lane: l, tick: tick_tag, start: t2, end: t3 });
        }
        let ns = (t3 - t0) as f64;
        rt.last_ns.store(t3 - t0, Ordering::Relaxed);
        if failure.is_none() {
            // execuções com falha (backtrace, relatório) não entram na média
            let avg = f64::from_bits(rt.avg_ns.load(Ordering::Relaxed));
            let avg = if rt.runs.fetch_add(1, Ordering::Relaxed) == 0 { ns } else { avg * 0.92 + ns * 0.08 };
            rt.avg_ns.store(avg.to_bits(), Ordering::Relaxed);
        }

        if let Some(msg) = failure {
            rt.failures.fetch_add(1, Ordering::Relaxed);
            match rt.decl.policy {
                FailPolicy::Abort => crash::fatal(&format!("sistema '{}' falhou (política: abortar)", rt.decl.name), &msg),
                _ => {
                    for (t, v) in snapshots {
                        sh.world.restore(t, v);
                    }
                    if rt.decl.policy != FailPolicy::SkipTick {
                        rt.enabled.store(false, Ordering::Relaxed);
                        crash::set_disabled(rt.id, true);
                    }
                    crash::system_failure(rt.decl.name, rt.decl.policy.name(), &msg);
                }
            }
        }
        crash::leave_system(lane, prev);
    }
}

struct RunShared<'w> {
    sched: &'w Schedule,
    world: &'w World,
    pool: &'w JobPool,
    info: TickInfo,
    opts: RunOpts,
    remaining: Vec<AtomicUsize>,
}

fn run_node<'a, 'w: 'a>(s: &Scope<'a>, sh: &'a RunShared<'w>, i: usize) {
    sh.sched.exec(i, sh);
    for &d in &sh.sched.systems[i].dependents {
        if sh.remaining[d].fetch_sub(1, Ordering::AcqRel) == 1 {
            s.spawn(move |s| run_node(s, sh, d));
        }
    }
}

// ---------------------------------------------------------------- acesso a dados

pub struct Read<'w, T> {
    g: RwLockReadGuard<'w, Erased>,
    _p: PhantomData<T>,
}

impl<T: 'static> Deref for Read<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        (**self.g).downcast_ref::<T>().expect("tipo de tabela")
    }
}

pub struct Write<'w, T> {
    g: RwLockWriteGuard<'w, Erased>,
    _p: PhantomData<T>,
}

impl<T: 'static> Deref for Write<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        (**self.g).downcast_ref::<T>().expect("tipo de tabela")
    }
}

impl<T: 'static> DerefMut for Write<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        (**self.g).downcast_mut::<T>().expect("tipo de tabela")
    }
}

pub struct SysCtx<'w> {
    pub world: &'w World,
    pub sys: &'w SysRt,
    pub info: TickInfo,
    pub pool: &'w JobPool,
    tick_tag: u8,
}

impl<'w> SysCtx<'w> {
    pub fn tick(&self) -> u64 {
        self.info.tick
    }
    pub fn dt(&self) -> f32 {
        self.info.dt
    }
    pub fn alpha(&self) -> f32 {
        self.info.alpha
    }
    pub fn id(&self) -> u16 {
        self.sys.id
    }

    fn mark(&self, table: usize) -> bool {
        match self.sys.access.iter().position(|a| a.table == table) {
            Some(k) => {
                self.sys.touched[k].store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn read<T: Table>(&self) -> Read<'w, T> {
        let id = self.world.id_of::<T>();
        if !self.mark(id) {
            panic!(
                "acesso não declarado: '{}' leu '{}' sem declarar em reads()",
                self.sys.decl.name, self.world.slots[id].decl.name
            );
        }
        let g = match self.world.slots[id].data.try_read() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => panic!(
                "conflito em runtime: '{}' tentou ler '{}' enquanto outro sistema escrevia",
                self.sys.decl.name, self.world.slots[id].decl.name
            ),
        };
        Read { g, _p: PhantomData }
    }

    pub fn write<T: Table>(&self) -> Write<'w, T> {
        let id = self.world.id_of::<T>();
        self.mark(id);
        if !self.sys.writes.contains(&id) {
            panic!(
                "escrita não declarada: '{}' escreveu em '{}' sem declarar em writes()",
                self.sys.decl.name, self.world.slots[id].decl.name
            );
        }
        let g = match self.world.slots[id].data.try_write() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => panic!(
                "conflito em runtime: '{}' tentou escrever em '{}', que já estava em uso",
                self.sys.decl.name, self.world.slots[id].decl.name
            ),
        };
        Write { g, _p: PhantomData }
    }

    /// Divide um framebuffer em faixas horizontais e desenha cada faixa num job.
    /// Cada faixa aparece como sub-zona do sistema na timeline.
    pub fn par_rows<F>(&self, px: &mut [u32], w: usize, h: usize, f: F)
    where
        F: Fn(&mut Canvas) + Sync,
    {
        let bands = (self.pool.lanes * 2).clamp(1, 64);
        let rows = h.div_ceil(bands).max(1);
        let name = self.sys.decl.name;
        let id = self.sys.id;
        let tick = self.tick_tag;
        let f = &f;
        self.pool.scope(|s| {
            for (i, chunk) in px.chunks_mut(rows * w).enumerate() {
                s.spawn(move |_| {
                    let lane = jobs::lane();
                    let prev = crash::enter_system(lane, id);
                    let t0 = now_ns();
                    let y0 = i * rows;
                    let y1 = y0 + chunk.len() / w;
                    let mut c = Canvas { w, h, y0, y1, px: chunk };
                    f(&mut c);
                    profiler::get().zone(Zone { name, sys: id, sub: i as u16, lane: lane as u8, tick, start: t0, end: now_ns() });
                    crash::leave_system(lane, prev);
                });
            }
        });
    }
}
