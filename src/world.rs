//! The world: entities in tables, plus global resources.
//!
//! Layout: every distinct *set* of component types gets its own `Table`, and a table stores one
//! `Vec<T>` per component (structure of arrays). Row `i` of every column belongs to entity
//! `entities[i]`. Systems never see entities one by one; they borrow whole columns as slices and
//! loop over them, which is what the cache and the vectorizer want.
//!
//! Safety model: while systems run they only get `&World`. Each column and resource sits in a
//! `SyncCell`, an atomic read/write lock that *panics* instead of blocking on conflict. The
//! scheduler orders systems by their declared access so conflicts never happen in practice; the
//! cell is the safety net that turns a wrong declaration into a clear panic instead of a data race.
//! Structural changes (spawn/despawn) need `&mut World` and are deferred through `Commands`.

use std::{
    any::{Any, TypeId, type_name},
    cell::UnsafeCell,
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::{
        Mutex,
        atomic::{AtomicIsize, Ordering},
    },
};

/// Anything that can live in a column or be a resource.
pub trait Component: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> Component for T {}

// ---------------------------------------------------------------------------------------------
// SyncCell: RefCell semantics, but atomic so it can be shared across worker threads.

pub struct SyncCell<T> {
    state: AtomicIsize, // >0: readers, -1: writer, 0: free
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is guarded by `state`, like RwLock.
unsafe impl<T: Send + Sync> Sync for SyncCell<T> {}

#[cold]
#[track_caller]
fn access_conflict<T>(wanted: &str) -> ! {
    panic!(
        "access conflict: tried to {wanted} `{}` while it is borrowed elsewhere. \
         Declare the access on the system (.reads::<T>() / .writes::<T>()) so the scheduler orders it.",
        type_name::<T>()
    )
}

impl<T> SyncCell<T> {
    pub fn new(value: T) -> SyncCell<T> {
        SyncCell { state: AtomicIsize::new(0), value: UnsafeCell::new(value) }
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    #[track_caller]
    pub fn borrow(&self) -> Ref<'_, T> {
        let mut s = self.state.load(Ordering::Relaxed);
        loop {
            if s < 0 {
                access_conflict::<T>("read");
            }
            match self.state.compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return Ref { cell: self },
                Err(actual) => s = actual,
            }
        }
    }

    #[track_caller]
    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        if self.state.compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed).is_err() {
            access_conflict::<T>("write");
        }
        RefMut { cell: self }
    }
}

pub struct Ref<'a, T> {
    cell: &'a SyncCell<T>,
}

impl<T> Deref for Ref<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: we hold a read lock.
        unsafe { &*self.cell.value.get() }
    }
}

impl<T> Drop for Ref<'_, T> {
    fn drop(&mut self) {
        self.cell.state.fetch_sub(1, Ordering::Release);
    }
}

pub struct RefMut<'a, T> {
    cell: &'a SyncCell<T>,
}

impl<T> Deref for RefMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: we hold the write lock.
        unsafe { &*self.cell.value.get() }
    }
}

impl<T> DerefMut for RefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: we hold the write lock.
        unsafe { &mut *self.cell.value.get() }
    }
}

impl<T> Drop for RefMut<'_, T> {
    fn drop(&mut self) {
        self.cell.state.store(0, Ordering::Release);
    }
}

pub type Res<'a, T> = Ref<'a, T>;
pub type ResMut<'a, T> = RefMut<'a, T>;

/// Read access to a whole column, as a slice.
pub struct ColRef<'a, T>(Ref<'a, Vec<T>>);

impl<T> Deref for ColRef<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}

/// Write access to a whole column, as a slice (the length can't change; that would desync rows).
pub struct ColMut<'a, T>(RefMut<'a, Vec<T>>);

impl<T> Deref for ColMut<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> DerefMut for ColMut<'_, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

// ---------------------------------------------------------------------------------------------
// Entities

/// Generational handle: `index` is reused after despawn, `generation` tells stale handles apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    pub fn index(self) -> u32 {
        self.index
    }
    pub fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy)]
struct Location {
    table: u32,
    row: u32,
}

struct Slot {
    generation: u32,
    location: Option<Location>,
}

// ---------------------------------------------------------------------------------------------
// Tables

#[doc(hidden)]
pub trait AnyColumn: Send + Sync {
    fn swap_remove(&mut self, row: usize);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn type_name(&self) -> &'static str;
    /// Heap bytes reserved by the column.
    fn heap_bytes(&self) -> usize;
}

#[doc(hidden)]
pub struct Column<T>(SyncCell<Vec<T>>);

impl<T: Component> Column<T> {
    pub fn boxed() -> Box<dyn AnyColumn> {
        Box::new(Column::<T>(SyncCell::new(Vec::new())))
    }
}

impl<T: Component> AnyColumn for Column<T> {
    fn swap_remove(&mut self, row: usize) {
        self.0.get_mut().swap_remove(row);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn type_name(&self) -> &'static str {
        type_name::<T>()
    }
    fn heap_bytes(&self) -> usize {
        self.0.borrow().capacity() * size_of::<T>()
    }
}

pub struct Table {
    types: Vec<TypeId>,
    columns: Vec<Box<dyn AnyColumn>>,
    entities: Vec<Entity>,
}

#[cold]
#[track_caller]
fn missing_column<T>() -> ! {
    panic!("table has no column `{}`", type_name::<T>())
}

impl Table {
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Entity of each row.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn has<T: Component>(&self) -> bool {
        self.types.contains(&TypeId::of::<T>())
    }

    pub fn component_names(&self) -> Vec<&'static str> {
        self.columns.iter().map(|c| c.type_name()).collect()
    }

    fn column<T: Component>(&self) -> Option<&Column<T>> {
        let i = self.types.iter().position(|&t| t == TypeId::of::<T>())?;
        self.columns[i].as_any().downcast_ref()
    }

    fn column_vec_mut<T: Component>(&mut self) -> &mut Vec<T> {
        let i = self.types.iter().position(|&t| t == TypeId::of::<T>()).unwrap_or_else(|| missing_column::<T>());
        self.columns[i].as_any_mut().downcast_mut::<Column<T>>().unwrap().0.get_mut()
    }

    #[track_caller]
    pub fn read<T: Component>(&self) -> ColRef<'_, T> {
        ColRef(self.column::<T>().unwrap_or_else(|| missing_column::<T>()).0.borrow())
    }

    #[track_caller]
    pub fn write<T: Component>(&self) -> ColMut<'_, T> {
        ColMut(self.column::<T>().unwrap_or_else(|| missing_column::<T>()).0.borrow_mut())
    }

    /// Exclusive access when you already hold `&mut World`; no locking.
    pub fn get_mut<T: Component>(&mut self) -> &mut [T] {
        self.column_vec_mut::<T>()
    }

    fn contains_all(&self, ids: &[TypeId]) -> bool {
        ids.iter().all(|id| self.types.contains(id))
    }
}

/// A set of component types, used to select tables: `world.query::<(Transform, Body)>()`.
pub trait ComponentSet: 'static {
    fn type_ids(out: &mut Vec<TypeId>);
}

/// A tuple of component values that can be spawned as one entity.
pub trait Bundle: ComponentSet + Send {
    #[doc(hidden)]
    fn new_columns(out: &mut Vec<(TypeId, Box<dyn AnyColumn>)>);
    #[doc(hidden)]
    fn push_into(self, table: &mut Table);
}

macro_rules! tuple_bundle {
    ($($name:ident $idx:tt),+) => {
        impl<$($name: Component),+> ComponentSet for ($($name,)+) {
            fn type_ids(out: &mut Vec<TypeId>) {
                $(out.push(TypeId::of::<$name>());)+
            }
        }
        impl<$($name: Component),+> Bundle for ($($name,)+) {
            fn new_columns(out: &mut Vec<(TypeId, Box<dyn AnyColumn>)>) {
                $(out.push((TypeId::of::<$name>(), Column::<$name>::boxed()));)+
            }
            fn push_into(self, table: &mut Table) {
                $(table.column_vec_mut::<$name>().push(self.$idx);)+
            }
        }
    };
}

tuple_bundle!(A 0);
tuple_bundle!(A 0, B 1);
tuple_bundle!(A 0, B 1, C 2);
tuple_bundle!(A 0, B 1, C 2, D 3);
tuple_bundle!(A 0, B 1, C 2, D 3, E 4);
tuple_bundle!(A 0, B 1, C 2, D 3, E 4, F 5);
tuple_bundle!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
tuple_bundle!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);

// ---------------------------------------------------------------------------------------------
// Commands

type DeferredFn = Box<dyn FnOnce(&mut World) + Send>;

/// Deferred structural changes. Each system owns one; they are applied after the schedule
/// finishes, in system order, so the result doesn't depend on which thread ran what.
#[derive(Default)]
pub struct Commands {
    queue: Mutex<Vec<DeferredFn>>,
}

impl Commands {
    pub fn spawn<B: Bundle>(&self, bundle: B) {
        self.run(move |w| {
            w.spawn(bundle);
        });
    }

    pub fn despawn(&self, e: Entity) {
        self.run(move |w| {
            w.despawn(e);
        });
    }

    /// Arbitrary deferred work with exclusive world access.
    pub fn run(&self, f: impl FnOnce(&mut World) + Send + 'static) {
        self.queue.lock().unwrap().push(Box::new(f));
    }

    pub fn apply(&mut self, world: &mut World) {
        for f in self.queue.get_mut().unwrap().drain(..) {
            f(world);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// World

#[derive(Default)]
pub struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
    tables: Vec<Table>,
    table_index: HashMap<Vec<TypeId>, u32>,
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

#[cold]
#[track_caller]
fn missing_resource<T>() -> ! {
    panic!("resource `{}` not found; insert it with World::insert_resource", type_name::<T>())
}

impl World {
    pub fn new() -> World {
        World::default()
    }

    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> Entity {
        let t = self.table_for::<B>();
        let table = &mut self.tables[t];
        let location = Location { table: t as u32, row: table.entities.len() as u32 };
        let entity = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.location = Some(location);
                Entity { index, generation: slot.generation }
            }
            None => {
                self.slots.push(Slot { generation: 0, location: Some(location) });
                Entity { index: self.slots.len() as u32 - 1, generation: 0 }
            }
        };
        bundle.push_into(table);
        table.entities.push(entity);
        entity
    }

    /// Removes the entity by swapping the table's last row into its place. Returns false if the
    /// handle was already dead.
    pub fn despawn(&mut self, e: Entity) -> bool {
        let Some(loc) = self.slot_location(e) else { return false };
        let table = &mut self.tables[loc.table as usize];
        let row = loc.row as usize;
        for c in &mut table.columns {
            c.swap_remove(row);
        }
        table.entities.swap_remove(row);
        if let Some(&moved) = table.entities.get(row) {
            self.slots[moved.index as usize].location = Some(loc);
        }
        let slot = &mut self.slots[e.index as usize];
        slot.location = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(e.index);
        true
    }

    fn slot_location(&self, e: Entity) -> Option<Location> {
        let slot = self.slots.get(e.index as usize)?;
        if slot.generation != e.generation {
            return None;
        }
        slot.location
    }

    pub fn is_alive(&self, e: Entity) -> bool {
        self.slot_location(e).is_some()
    }

    /// (table index, row) of a live entity.
    pub fn location(&self, e: Entity) -> Option<(usize, usize)> {
        self.slot_location(e).map(|l| (l.table as usize, l.row as usize))
    }

    pub fn entity_count(&self) -> usize {
        self.tables.iter().map(|t| t.len()).sum()
    }

    /// Copy of one component of one entity. For bulk work, borrow columns instead.
    pub fn get<T: Component + Clone>(&self, e: Entity) -> Option<T> {
        let (t, row) = self.location(e)?;
        let table = &self.tables[t];
        if !table.has::<T>() {
            return None;
        }
        Some(table.read::<T>()[row].clone())
    }

    /// Heap bytes reserved by component columns and entity bookkeeping (resources not included).
    pub fn memory(&self) -> Vec<(&'static str, usize)> {
        let columns = self.tables.iter().flat_map(|t| t.columns.iter()).map(|c| c.heap_bytes()).sum();
        let entities = self.slots.capacity() * size_of::<Slot>()
            + self.free.capacity() * 4
            + self.tables.iter().map(|t| t.entities.capacity() * size_of::<Entity>()).sum::<usize>();
        vec![("world.components", columns), ("world.entities", entities)]
    }

    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    /// Non-empty tables that contain every component of `Q`.
    pub fn query<Q: ComponentSet>(&self) -> impl Iterator<Item = &Table> + '_ {
        let mut ids = Vec::new();
        Q::type_ids(&mut ids);
        self.tables.iter().filter(move |t| !t.is_empty() && t.contains_all(&ids))
    }

    fn table_for<B: Bundle>(&mut self) -> usize {
        let mut ids = Vec::new();
        B::type_ids(&mut ids);
        ids.sort();
        if let Some(&t) = self.table_index.get(&ids) {
            return t as usize;
        }
        assert!(ids.windows(2).all(|w| w[0] != w[1]), "bundle contains the same component twice");
        let mut columns = Vec::new();
        B::new_columns(&mut columns);
        columns.sort_by_key(|(id, _)| *id);
        let t = self.tables.len();
        self.tables.push(Table {
            types: columns.iter().map(|(id, _)| *id).collect(),
            columns: columns.into_iter().map(|(_, c)| c).collect(),
            entities: Vec::new(),
        });
        self.table_index.insert(ids, t as u32);
        t
    }

    // --- resources ---------------------------------------------------------------------------

    pub fn insert_resource<T: Component>(&mut self, value: T) {
        self.resources.insert(TypeId::of::<T>(), Box::new(SyncCell::new(value)));
    }

    pub fn has_resource<T: Component>(&self) -> bool {
        self.resources.contains_key(&TypeId::of::<T>())
    }

    fn resource_cell<T: Component>(&self) -> Option<&SyncCell<T>> {
        self.resources.get(&TypeId::of::<T>()).map(|b| b.downcast_ref::<SyncCell<T>>().unwrap())
    }

    #[track_caller]
    pub fn res<T: Component>(&self) -> Res<'_, T> {
        self.resource_cell::<T>().unwrap_or_else(|| missing_resource::<T>()).borrow()
    }

    #[track_caller]
    pub fn res_mut<T: Component>(&self) -> ResMut<'_, T> {
        self.resource_cell::<T>().unwrap_or_else(|| missing_resource::<T>()).borrow_mut()
    }

    pub fn try_res<T: Component>(&self) -> Option<Res<'_, T>> {
        self.resource_cell::<T>().map(|c| c.borrow())
    }

    /// Exclusive access when you hold `&mut World`; no locking.
    #[track_caller]
    pub fn resource_mut<T: Component>(&mut self) -> &mut T {
        self.resources
            .get_mut(&TypeId::of::<T>())
            .unwrap_or_else(|| missing_resource::<T>())
            .downcast_mut::<SyncCell<T>>()
            .unwrap()
            .get_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Pos(f32);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Vel(f32);

    #[test]
    fn spawn_query_despawn() {
        let mut w = World::new();
        let a = w.spawn((Pos(1.0), Vel(10.0)));
        let b = w.spawn((Vel(20.0), Pos(2.0))); // same table, any order
        let c = w.spawn((Pos(3.0),));
        assert_eq!(w.query::<(Pos,)>().count(), 2);
        assert_eq!(w.query::<(Pos, Vel)>().count(), 1);

        assert!(w.despawn(a));
        assert!(!w.despawn(a));
        assert!(!w.is_alive(a) && w.is_alive(b) && w.is_alive(c));
        assert_eq!(w.get::<Pos>(b), Some(Pos(2.0))); // b was moved into a's row
        assert_eq!(w.location(b).unwrap().1, 0);

        let d = w.spawn((Pos(4.0), Vel(0.0)));
        assert_eq!(d.index(), a.index());
        assert_ne!(d, a);
    }

    #[test]
    fn columns_borrow_as_slices() {
        let mut w = World::new();
        for i in 0..100 {
            w.spawn((Pos(i as f32), Vel(1.0)));
        }
        let t = w.query::<(Pos, Vel)>().next().unwrap();
        let mut p = t.write::<Pos>();
        let v = t.read::<Vel>();
        for (p, v) in p.iter_mut().zip(v.iter()) {
            p.0 += v.0;
        }
        assert_eq!(p[99], Pos(100.0));
    }

    #[test]
    #[should_panic(expected = "access conflict")]
    fn conflicting_borrows_panic() {
        let mut w = World::new();
        w.spawn((Pos(0.0),));
        let t = w.query::<(Pos,)>().next().unwrap();
        let _r = t.read::<Pos>();
        let _w = t.write::<Pos>();
    }

    #[test]
    fn commands_apply_in_order() {
        let mut w = World::new();
        let mut cmds = Commands::default();
        cmds.spawn((Pos(1.0),));
        cmds.run(|w| w.insert_resource(7u32));
        cmds.apply(&mut w);
        assert_eq!(w.entity_count(), 1);
        assert_eq!(*w.res::<u32>(), 7);
    }
}
