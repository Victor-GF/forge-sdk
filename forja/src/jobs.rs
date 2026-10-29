//! Job system feito à mão: um pool fixo de workers e uma fila única.
//!
//! `scope` permite jobs que pegam dados emprestados da pilha (como `std::thread::scope`),
//! sem criar threads por frame. Quem espera um scope ajuda a executar jobs da fila,
//! então jobs podem abrir scopes aninhados sem deadlock.

use std::cell::Cell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

type Job = Box<dyn FnOnce() + Send + 'static>;

struct State {
    jobs: VecDeque<Job>,
    shutdown: bool,
}

struct Shared {
    q: Mutex<State>,
    cv: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.q.lock().unwrap_or_else(|e| e.into_inner())
    }
}

thread_local! {
    static LANE: Cell<usize> = const { Cell::new(0) };
}

/// Índice da "lane" da thread atual: 0 = main, 1..N = workers.
pub fn lane() -> usize {
    LANE.with(|l| l.get())
}

pub struct JobPool {
    shared: Arc<Shared>,
    threads: Vec<std::thread::JoinHandle<()>>,
    pub lanes: usize,
}

struct SendPtr(*const ());
unsafe impl Send for SendPtr {}
impl SendPtr {
    fn get(&self) -> *const () {
        self.0
    }
}

impl JobPool {
    pub fn new(workers: usize) -> JobPool {
        let shared = Arc::new(Shared { q: Mutex::new(State { jobs: VecDeque::new(), shutdown: false }), cv: Condvar::new() });
        let threads = (0..workers)
            .map(|i| {
                let s = shared.clone();
                std::thread::Builder::new()
                    .name(format!("worker-{}", i + 1))
                    .spawn(move || {
                        LANE.with(|l| l.set(i + 1));
                        crate::crash::register_lane_thread(i + 1);
                        worker_loop(&s);
                    })
                    .expect("falha ao criar worker")
            })
            .collect();
        crate::crash::register_lane_thread(0);
        JobPool { shared, threads, lanes: workers + 1 }
    }

    fn push(&self, job: Job) {
        self.shared.lock().jobs.push_back(job);
        self.shared.cv.notify_all();
    }

    /// Executa `f` e espera todos os jobs criados nele. Se algum job entrar em pânico,
    /// o pânico é repassado para quem abriu o scope (o sistema dono do trabalho).
    pub fn scope<'a, R>(&'a self, f: impl FnOnce(&Scope<'a>) -> R) -> R {
        let s = Scope { pool: self, pending: AtomicUsize::new(0), panic: Mutex::new(None), _m: PhantomData };
        let r = f(&s);
        s.wait();
        if let Some(msg) = s.panic.lock().unwrap_or_else(|e| e.into_inner()).take() {
            panic!("sub-job entrou em pânico: {}", msg);
        }
        r
        // Drop de `s` também espera, caso `f` entre em pânico.
    }
}

fn worker_loop(s: &Shared) {
    loop {
        let job = {
            let mut st = s.lock();
            loop {
                if let Some(j) = st.jobs.pop_front() {
                    break Some(j);
                }
                if st.shutdown {
                    break None;
                }
                st = s.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        match job {
            Some(j) => j(),
            None => return,
        }
    }
}

impl Drop for JobPool {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.cv.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

pub struct Scope<'a> {
    pool: &'a JobPool,
    pending: AtomicUsize,
    panic: Mutex<Option<String>>,
    _m: PhantomData<fn(&'a ()) -> &'a ()>,
}

impl<'a> Scope<'a> {
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce(&Scope<'a>) + Send + 'a,
    {
        self.pending.fetch_add(1, Ordering::AcqRel);
        let sp = SendPtr(self as *const Scope<'a> as *const ());
        let sh = SendPtr(&*self.pool.shared as *const Shared as *const ());
        let job: Box<dyn FnOnce() + Send + 'a> = Box::new(move || {
            // SAFETY: o dono do scope só retorna depois de ver pending == 0, e o
            // decremento acontece sob o lock abaixo, então `s` é válido até lá.
            let s = unsafe { &*(sp.get() as *const Scope<'a>) };
            let shared = unsafe { &*(sh.get() as *const Shared) };
            if let Err(p) = catch_unwind(AssertUnwindSafe(|| f(s))) {
                let mut slot = s.panic.lock().unwrap_or_else(|e| e.into_inner());
                if slot.is_none() {
                    *slot = Some(crate::crash::payload_str(&*p));
                }
            }
            let g = shared.lock();
            s.pending.fetch_sub(1, Ordering::AcqRel);
            drop(g);
            shared.cv.notify_all();
        });
        // SAFETY: o scope espera todos os jobs antes de sair, então os empréstimos 'a
        // continuam válidos durante a execução.
        let job: Job = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + 'a>, Job>(job) };
        self.pool.push(job);
    }

    fn wait(&self) {
        let shared = &*self.pool.shared;
        let mut st = shared.lock();
        loop {
            if self.pending.load(Ordering::Acquire) == 0 {
                return;
            }
            if let Some(j) = st.jobs.pop_front() {
                drop(st);
                j();
                st = shared.lock();
            } else {
                st = shared.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.wait();
    }
}
