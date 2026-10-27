//! Systems and schedules.
//!
//! A **system** is a plain function `fn(&SystemCtx)` plus a declaration of the data it touches:
//!
//! ```ignore
//! system("physics.sync", sync).reads::<PhysicsWorld>().reads::<Body>().writes::<Transform>()
//! ```
//!
//! A **schedule** is an ordered list of systems. Order of insertion is the logical order; from it
//! and the declarations the schedule derives a DAG: system B waits for an earlier system A only
//! when they conflict (one writes what the other reads or writes) or B says `.after("A")`.
//! Everything else runs concurrently. Inside a system, data parallelism comes from `ctx.jobs`
//! (`par_zip` over column slices). So: coarse systems for task parallelism, batches for data
//! parallelism, and the dependency graph is computed, never hand-wired.

use std::any::{TypeId, type_name};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    frame::FrameInfo,
    jobs::{Jobs, Scope},
    profile,
    world::{ColMut, ColRef, Commands, Component, ComponentSet, Entity, Res, ResMut, Table, World},
};

#[derive(Default, Clone)]
struct Access {
    reads: Vec<(TypeId, &'static str)>,
    writes: Vec<(TypeId, &'static str)>,
}

impl Access {
    fn can_read(&self, id: TypeId) -> bool {
        self.reads.iter().chain(&self.writes).any(|(t, _)| *t == id)
    }
    fn can_write(&self, id: TypeId) -> bool {
        self.writes.iter().any(|(t, _)| *t == id)
    }
    fn conflicts(&self, o: &Access) -> bool {
        self.writes.iter().any(|(t, _)| o.can_read(*t)) || o.writes.iter().any(|(t, _)| self.can_read(*t))
    }
}

type SystemFn = Box<dyn Fn(&SystemCtx) + Send + Sync>;

pub struct System {
    name: &'static str,
    func: SystemFn,
    access: Access,
    after: Vec<&'static str>,
    commands: Commands,
}

/// Creates a system. Chain `.reads::<T>()`, `.writes::<T>()` (components or resources) and
/// `.after("other")` to declare its access and explicit ordering.
pub fn system(name: &'static str, f: impl Fn(&SystemCtx) + Send + Sync + 'static) -> System {
    System { name, func: Box::new(f), access: Access::default(), after: Vec::new(), commands: Commands::default() }
}

impl System {
    pub fn reads<T: 'static>(mut self) -> System {
        self.access.reads.push((TypeId::of::<T>(), type_name::<T>()));
        self
    }
    pub fn writes<T: 'static>(mut self) -> System {
        self.access.writes.push((TypeId::of::<T>(), type_name::<T>()));
        self
    }
    pub fn after(mut self, name: &'static str) -> System {
        self.after.push(name);
        self
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// What a running system sees. All data access goes through it so undeclared access is caught.
pub struct SystemCtx<'a> {
    world: &'a World,
    system: &'a System,
    pub jobs: &'a Jobs,
    pub frame: &'a FrameInfo,
    /// Deferred spawns/despawns, applied when the schedule finishes.
    pub commands: &'a Commands,
}

#[cold]
#[track_caller]
fn undeclared<T>(system: &str, how: &str) -> ! {
    panic!("system `{system}` accessed `{}` without declaring .{how}::<{}>()", type_name::<T>(), type_name::<T>())
}

impl<'a> SystemCtx<'a> {
    pub fn name(&self) -> &'static str {
        self.system.name
    }

    #[track_caller]
    pub fn res<T: Component>(&self) -> Res<'a, T> {
        if !self.system.access.can_read(TypeId::of::<T>()) {
            undeclared::<T>(self.system.name, "reads");
        }
        self.world.res::<T>()
    }

    #[track_caller]
    pub fn res_mut<T: Component>(&self) -> ResMut<'a, T> {
        if !self.system.access.can_write(TypeId::of::<T>()) {
            undeclared::<T>(self.system.name, "writes");
        }
        self.world.res_mut::<T>()
    }

    /// Tables containing all components of `Q`. Borrow their columns with `read`/`write`.
    pub fn query<Q: ComponentSet>(&self) -> impl Iterator<Item = TableRef<'a>> + 'a {
        let system = self.system;
        self.world.query::<Q>().map(move |table| TableRef { table, system })
    }

    /// Is the entity still alive? Reading entity liveness needs no declaration.
    pub fn is_alive(&self, e: Entity) -> bool {
        self.world.is_alive(e)
    }
}

/// A table as seen from inside a system: same API, plus access checks.
#[derive(Clone, Copy)]
pub struct TableRef<'a> {
    table: &'a Table,
    system: &'a System,
}

impl<'a> TableRef<'a> {
    pub fn len(&self) -> usize {
        self.table.len()
    }
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
    pub fn entities(&self) -> &'a [Entity] {
        self.table.entities()
    }
    pub fn has<T: Component>(&self) -> bool {
        self.table.has::<T>()
    }
    #[track_caller]
    pub fn read<T: Component>(&self) -> ColRef<'a, T> {
        if !self.system.access.can_read(TypeId::of::<T>()) {
            undeclared::<T>(self.system.name, "reads");
        }
        self.table.read::<T>()
    }
    #[track_caller]
    pub fn write<T: Component>(&self) -> ColMut<'a, T> {
        if !self.system.access.can_write(TypeId::of::<T>()) {
            undeclared::<T>(self.system.name, "writes");
        }
        self.table.write::<T>()
    }
}

pub struct Schedule {
    name: &'static str,
    systems: Vec<System>,
    /// dependents[i]: systems that wait for system i.
    dependents: Vec<Vec<usize>>,
    dependency_count: Vec<usize>,
    built: bool,
}

impl Schedule {
    pub fn new(name: &'static str) -> Schedule {
        Schedule { name, systems: Vec::new(), dependents: Vec::new(), dependency_count: Vec::new(), built: false }
    }

    pub fn add(&mut self, s: System) -> &mut Schedule {
        assert!(
            self.systems.iter().all(|o| o.name != s.name),
            "schedule `{}` already has a system named `{}`",
            self.name,
            s.name
        );
        for a in &s.after {
            assert!(
                self.systems.iter().any(|o| o.name == *a),
                "system `{}` runs after `{a}`, which must be added to schedule `{}` first",
                s.name,
                self.name
            );
        }
        self.systems.push(s);
        self.built = false;
        self
    }

    pub fn add_all(&mut self, systems: impl IntoIterator<Item = System>) -> &mut Schedule {
        for s in systems {
            self.add(s);
        }
        self
    }

    fn build(&mut self) {
        let n = self.systems.len();
        self.dependents = vec![Vec::new(); n];
        self.dependency_count = vec![0; n];
        for j in 0..n {
            for i in 0..j {
                let (a, b) = (&self.systems[i], &self.systems[j]);
                if a.access.conflicts(&b.access) || b.after.contains(&a.name) {
                    self.dependents[i].push(j);
                    self.dependency_count[j] += 1;
                }
            }
        }
        self.built = true;
    }

    /// Runs every system once, in parallel where the graph allows, then applies their commands
    /// in system order.
    pub fn run(&mut self, world: &mut World, jobs: &Jobs, frame: &FrameInfo) {
        if !self.built {
            self.build();
        }
        {
            let _frame = profile::frame_tag(frame.frame);
            let _p = profile::scope_kind(profile::Kind::Schedule, self.name);
            let exec = Executor {
                systems: &self.systems,
                dependents: &self.dependents,
                remaining: self.dependency_count.iter().map(|&c| AtomicUsize::new(c)).collect(),
                world,
                jobs,
                frame,
            };
            jobs.scope(|s| {
                for (i, &count) in self.dependency_count.iter().enumerate() {
                    if count == 0 {
                        exec.launch(s, i);
                    }
                }
            });
        }
        for s in &mut self.systems {
            s.commands.apply(world);
        }
    }

    /// Human-readable view of the graph: each system's wave (systems in the same wave can run
    /// together), its access, and what it waits for.
    pub fn describe(&mut self) -> String {
        if !self.built {
            self.build();
        }
        let n = self.systems.len();
        let mut wave = vec![0usize; n];
        let mut waits: Vec<Vec<&str>> = vec![Vec::new(); n];
        for i in 0..n {
            for &d in &self.dependents[i] {
                wave[d] = wave[d].max(wave[i] + 1);
                waits[d].push(self.systems[i].name);
            }
        }
        let short = |list: &[(TypeId, &'static str)]| {
            list.iter().map(|(_, n)| n.rsplit("::").next().unwrap_or(n)).collect::<Vec<_>>().join(", ")
        };
        let mut out = format!("schedule `{}`\n", self.name);
        for (i, s) in self.systems.iter().enumerate() {
            out += &format!(
                "  wave {}  {:<22} reads [{}] writes [{}]{}\n",
                wave[i],
                s.name,
                short(&s.access.reads),
                short(&s.access.writes),
                if waits[i].is_empty() { String::new() } else { format!("  after {}", waits[i].join(", ")) }
            );
        }
        out
    }
}

struct Executor<'a> {
    systems: &'a [System],
    dependents: &'a [Vec<usize>],
    remaining: Vec<AtomicUsize>,
    world: &'a World,
    jobs: &'a Jobs,
    frame: &'a FrameInfo,
}

impl Executor<'_> {
    fn launch<'scope, 'env>(&'scope self, scope: &'scope Scope<'scope, 'env>, i: usize) {
        scope.spawn(move || {
            let system = &self.systems[i];
            {
                let _p = profile::scope_kind(profile::Kind::System, system.name);
                let ctx = SystemCtx {
                    world: self.world,
                    system,
                    jobs: self.jobs,
                    frame: self.frame,
                    commands: &system.commands,
                };
                (system.func)(&ctx);
            }
            for &d in &self.dependents[i] {
                if self.remaining[d].fetch_sub(1, Ordering::AcqRel) == 1 {
                    self.launch(scope, d);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobSystem;
    use std::sync::Mutex;

    struct A(u32);
    struct B;
    struct Log(Mutex<Vec<&'static str>>);

    #[test]
    fn conflicts_become_edges_and_independent_systems_share_a_wave() {
        let mut s = Schedule::new("test");
        s.add(system("write_a", |_| {}).writes::<A>());
        s.add(system("write_b", |_| {}).writes::<B>());
        s.add(system("read_a", |_| {}).reads::<A>());
        s.add(system("read_ab", |_| {}).reads::<A>().reads::<B>());
        let d = s.describe();
        assert!(d.contains("wave 0  write_a"), "{d}");
        assert!(d.contains("wave 0  write_b"), "{d}");
        assert!(d.contains("wave 1  read_a"), "{d}");
        assert!(d.contains("wave 1  read_ab"), "{d}");
    }

    #[test]
    fn ordering_and_commands() {
        let jobs = JobSystem::new(3);
        let mut world = World::new();
        world.insert_resource(A(0));
        world.insert_resource(Log(Mutex::new(Vec::new())));
        let mut s = Schedule::new("test");
        s.add(system("inc", |ctx| ctx.res_mut::<A>().0 += 1).writes::<A>());
        s.add(
            system("check", |ctx| {
                // main thread made it odd, "inc" (which conflicts, so runs first) made it even
                assert_eq!(ctx.res::<A>().0 % 2, 0);
                ctx.res::<Log>().0.lock().unwrap().push("check");
            })
            .reads::<A>()
            .reads::<Log>(),
        );
        s.add(system("spawn", |ctx| ctx.commands.spawn((B,))).after("check"));
        for _ in 0..50 {
            world.resource_mut::<A>().0 += 1;
            s.run(&mut world, &jobs, &FrameInfo::default());
        }
        assert_eq!(world.entity_count(), 50);
        assert_eq!(world.res::<Log>().0.lock().unwrap().len(), 50);
    }

    #[test]
    #[should_panic(expected = "without declaring .writes")]
    fn undeclared_access_panics() {
        let jobs = JobSystem::new(1);
        let mut world = World::new();
        world.insert_resource(A(0));
        let mut s = Schedule::new("test");
        s.add(system("sneaky", |ctx| ctx.res_mut::<A>().0 = 1).reads::<A>());
        s.run(&mut world, &jobs, &FrameInfo::default());
    }
}
