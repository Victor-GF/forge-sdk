//! O "banco de dados": tabelas tipadas com metadados declarados (colunas, layout,
//! tempo de vida, invariantes). O runtime acessa os dados diretamente; os metadados
//! existem para o scheduler, para a análise e para o editor.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lifetime {
    /// Cozida offline / na carga; só leitura em runtime.
    Static,
    /// Vive a partida inteira.
    Session,
    /// Zerada no início de cada tick (arena de frame).
    Transient,
}

impl Lifetime {
    pub fn name(self) -> &'static str {
        match self {
            Lifetime::Static => "estática",
            Lifetime::Session => "sessão",
            Lifetime::Transient => "transitória",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    SoA,
    AoS,
}

pub trait Table: Any + Send + Sync + Clone {
    fn rows(&self) -> usize;
    fn clear_transient(&mut self) {}
}

#[derive(Clone, Debug)]
pub struct Column {
    pub name: &'static str,
    pub bytes: usize,
}

#[derive(Clone, Debug)]
pub struct TableDecl {
    pub name: &'static str,
    pub lifetime: Lifetime,
    pub layout: Layout,
    pub columns: Vec<Column>,
    /// Tamanho real da linha em memória (AoS: size_of, com padding).
    pub row_size: usize,
    /// Acesso por amostragem/consulta, não varredura: estimativas de cache não se aplicam.
    pub sparse: bool,
    /// Consumida fora do grafo (ex.: framebuffer lido pela plataforma).
    pub external: bool,
    pub note: &'static str,
}

impl TableDecl {
    pub fn soa(name: &'static str, lifetime: Lifetime) -> TableDecl {
        TableDecl { name, lifetime, layout: Layout::SoA, columns: Vec::new(), row_size: 0, sparse: false, external: false, note: "" }
    }

    pub fn aos<Row>(name: &'static str, lifetime: Lifetime) -> TableDecl {
        TableDecl { row_size: std::mem::size_of::<Row>(), layout: Layout::AoS, ..TableDecl::soa(name, lifetime) }
    }

    pub fn col(mut self, name: &'static str, bytes: usize) -> TableDecl {
        self.columns.push(Column { name, bytes });
        self
    }

    pub fn sparse(mut self) -> TableDecl {
        self.sparse = true;
        self
    }

    pub fn external(mut self) -> TableDecl {
        self.external = true;
        self
    }

    pub fn note(mut self, note: &'static str) -> TableDecl {
        self.note = note;
        self
    }

    pub fn used_bytes(&self) -> usize {
        self.columns.iter().map(|c| c.bytes).sum()
    }

    pub fn row_bytes(&self) -> usize {
        match self.layout {
            Layout::SoA => self.used_bytes(),
            Layout::AoS => self.row_size.max(self.used_bytes()),
        }
    }

    pub fn col_bytes(&self, name: &str) -> Option<usize> {
        self.columns.iter().find(|c| c.name == name).map(|c| c.bytes)
    }
}

pub type Erased = Box<dyn Any + Send + Sync>;
type Invariant = Box<dyn Fn(&dyn Any) -> Result<(), String> + Send + Sync>;

pub struct Slot {
    pub decl: TableDecl,
    pub data: RwLock<Erased>,
    rows_fn: fn(&dyn Any) -> usize,
    clone_fn: fn(&dyn Any) -> Erased,
    clear_fn: fn(&mut dyn Any),
    invariants: Vec<(&'static str, Invariant)>,
}

impl Slot {
    pub fn invariant_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.invariants.iter().map(|(n, _)| *n)
    }
}

fn rows_of<T: Table>(a: &dyn Any) -> usize {
    a.downcast_ref::<T>().map(|t| t.rows()).unwrap_or(0)
}
fn clone_of<T: Table>(a: &dyn Any) -> Erased {
    Box::new(a.downcast_ref::<T>().expect("tipo de tabela").clone())
}
fn clear_of<T: Table>(a: &mut dyn Any) {
    if let Some(t) = a.downcast_mut::<T>() {
        t.clear_transient();
    }
}

#[derive(Default)]
pub struct World {
    pub slots: Vec<Slot>,
    by_type: HashMap<TypeId, usize>,
}

impl World {
    pub fn new() -> World {
        World::default()
    }

    pub fn add<T: Table>(&mut self, decl: TableDecl, value: T) -> usize {
        assert!(!self.by_type.contains_key(&TypeId::of::<T>()), "tabela registrada duas vezes: {}", decl.name);
        let id = self.slots.len();
        self.slots.push(Slot {
            decl,
            data: RwLock::new(Box::new(value)),
            rows_fn: rows_of::<T>,
            clone_fn: clone_of::<T>,
            clear_fn: clear_of::<T>,
            invariants: Vec::new(),
        });
        self.by_type.insert(TypeId::of::<T>(), id);
        id
    }

    pub fn invariant<T: Table>(&mut self, name: &'static str, f: impl Fn(&T) -> Result<(), String> + Send + Sync + 'static) {
        let id = self.id_of::<T>();
        self.slots[id].invariants.push((name, Box::new(move |a: &dyn Any| f(a.downcast_ref::<T>().expect("tipo")))));
    }

    pub fn try_id(&self, ty: TypeId) -> Option<usize> {
        self.by_type.get(&ty).copied()
    }

    pub fn id_by_name(&self, name: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.decl.name == name)
    }

    pub fn id_of<T: 'static>(&self) -> usize {
        match self.by_type.get(&TypeId::of::<T>()) {
            Some(&id) => id,
            None => panic!("tabela não registrada: {}", std::any::type_name::<T>()),
        }
    }

    /// Acesso exclusivo fora do grafo (main thread, entre execuções).
    pub fn get_mut<T: Table>(&mut self) -> &mut T {
        let id = self.id_of::<T>();
        let data = self.slots[id].data.get_mut().unwrap_or_else(|e| e.into_inner());
        (**data).downcast_mut::<T>().expect("tipo")
    }

    pub fn get<T: Table>(&mut self) -> &T {
        self.get_mut::<T>()
    }

    /// Leitura compartilhada fora do grafo (ex.: editor). Não chamar com o grafo rodando.
    pub fn with<T: Table, R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let id = self.id_of::<T>();
        let g = self.slots[id].data.read().unwrap_or_else(|e| e.into_inner());
        f((**g).downcast_ref::<T>().expect("tipo"))
    }

    pub fn rows(&self, id: usize) -> usize {
        let slot = &self.slots[id];
        match slot.data.try_read() {
            Ok(g) => (slot.rows_fn)(&**g),
            Err(std::sync::TryLockError::Poisoned(e)) => (slot.rows_fn)(&**e.into_inner()),
            Err(_) => 0,
        }
    }

    pub fn total_bytes(&self, id: usize) -> usize {
        self.rows(id) * self.slots[id].decl.row_bytes()
    }

    /// Cópia completa da tabela: base das escritas transacionais.
    pub fn snapshot(&self, id: usize) -> Erased {
        let slot = &self.slots[id];
        let g = slot.data.read().unwrap_or_else(|e| e.into_inner());
        (slot.clone_fn)(&**g)
    }

    pub fn restore(&self, id: usize, value: Erased) {
        let slot = &self.slots[id];
        {
            let mut g = slot.data.write().unwrap_or_else(|e| e.into_inner());
            *g = value;
        }
        slot.data.clear_poison();
    }

    pub fn has_invariants(&self, id: usize) -> bool {
        !self.slots[id].invariants.is_empty()
    }

    pub fn check(&self, id: usize) -> Result<(), String> {
        let slot = &self.slots[id];
        if slot.invariants.is_empty() {
            return Ok(());
        }
        let g = slot.data.read().unwrap_or_else(|e| e.into_inner());
        for (name, f) in &slot.invariants {
            f(&**g).map_err(|e| format!("[{}] {}", name, e))?;
        }
        Ok(())
    }

    pub fn clear_transients(&mut self) {
        for slot in &mut self.slots {
            if slot.decl.lifetime == Lifetime::Transient {
                let data = slot.data.get_mut().unwrap_or_else(|e| e.into_inner());
                (slot.clear_fn)(&mut **data);
            }
        }
    }
}
