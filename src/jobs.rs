//! Job system: a fixed pool of worker threads fed from two FIFO queues.
//!
//! - **frame** queue: scoped jobs (`scope`, `par_for`, `par_zip`, schedule systems). These are the
//!   current frame's critical path.
//! - **background** queue: detached jobs (`spawn`), e.g. rendering the previous frame. Only idle
//!   workers (and `JobHandle::join`) pick these, so a short wait never gets stuck inside a long job.
//!
//! A thread that waits on a scope helps, but only with **that scope's own jobs** (task isolation,
//! like TBB's `this_task_arena::isolate`). Helping with anything would let the main thread, waiting
//! for a 50 µs physics batch, pick up a 5 ms raster strip of the frame being rendered in the
//! background and only come back when it's done. Telemetry showed exactly that: with 2 threads the
//! main thread ran ~19 ms/frame of render work inside physics passes. Idle workers still take any
//! job, so nothing is left unrun; nested scopes are covered because their own waiters help them.
//!
//! Thread 0 is the thread that created the `JobSystem` (the main thread); workers are 1..n.

use std::{
    any::Any,
    cell::Cell,
    collections::VecDeque,
    marker::PhantomData,
    ops::{Deref, Range},
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
};

use crate::profile;

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct Queues {
    /// (scope tag, job).
    frame: VecDeque<(usize, Job)>,
    background: VecDeque<Job>,
}

/// Which queued jobs a waiting thread may run.
#[derive(Clone, Copy)]
enum Help {
    /// Only jobs spawned on the scope with this tag.
    Scope(usize),
    /// Any frame job, and background jobs too.
    Any,
}

struct Shared {
    queues: Mutex<Queues>,
    wake: Condvar,
    /// Number of jobs in the frame queue; lets waiters skip the lock when there is nothing to steal.
    frame_queued: AtomicUsize,
    shutdown: AtomicBool,
    thread_count: usize,
    /// Jobs executed by each thread since start (telemetry).
    jobs_run: Vec<AtomicU64>,
}

thread_local! {
    static THREAD_INDEX: Cell<usize> = const { Cell::new(0) };
}

/// Index of the calling thread inside the pool: 0 for the main thread, 1..n for workers.
pub fn thread_index() -> usize {
    THREAD_INDEX.with(|t| t.get())
}

/// Cheap, cloneable handle to the pool. Everything that submits work takes a `&Jobs`.
#[derive(Clone)]
pub struct Jobs {
    shared: Arc<Shared>,
}

/// Owns the worker threads. Dropping it stops and joins them.
pub struct JobSystem {
    jobs: Jobs,
    threads: Vec<JoinHandle<()>>,
}

impl JobSystem {
    /// `workers` extra threads; the calling thread also executes jobs while it waits.
    pub fn new(workers: usize) -> JobSystem {
        crate::cpu::check_floor();
        let shared = Arc::new(Shared {
            queues: Mutex::new(Queues::default()),
            wake: Condvar::new(),
            frame_queued: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            thread_count: workers + 1,
            jobs_run: (0..=workers).map(|_| AtomicU64::new(0)).collect(),
        });
        THREAD_INDEX.with(|t| t.set(0));
        let threads = (1..=workers)
            .map(|index| {
                let shared = shared.clone();
                thread::Builder::new()
                    .name(format!("forja-worker-{index}"))
                    .spawn(move || worker_main(shared, index))
                    .expect("failed to spawn worker thread")
            })
            .collect();
        JobSystem { jobs: Jobs { shared }, threads }
    }

    /// One worker per hardware thread, minus the calling thread.
    pub fn with_available_parallelism() -> JobSystem {
        let n = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        JobSystem::new(n.saturating_sub(1))
    }

    pub fn handle(&self) -> Jobs {
        self.jobs.clone()
    }
}

impl Deref for JobSystem {
    type Target = Jobs;
    fn deref(&self) -> &Jobs {
        &self.jobs
    }
}

impl Drop for JobSystem {
    fn drop(&mut self) {
        let shared = &self.jobs.shared;
        shared.shutdown.store(true, Ordering::Release);
        drop(shared.queues.lock().unwrap());
        shared.wake.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn worker_main(shared: Arc<Shared>, index: usize) {
    THREAD_INDEX.with(|t| t.set(index));
    loop {
        let job = {
            let mut q = shared.queues.lock().unwrap();
            loop {
                if let Some((_, job)) = q.frame.pop_front() {
                    shared.frame_queued.fetch_sub(1, Ordering::Relaxed);
                    break Some(job);
                }
                if let Some(job) = q.background.pop_front() {
                    break Some(job);
                }
                if shared.shutdown.load(Ordering::Acquire) {
                    break None;
                }
                let _idle = profile::scope(profile::IDLE);
                q = shared.wake.wait(q).unwrap();
            }
        };
        match job {
            // Jobs catch their own panics, so a worker never unwinds.
            Some(job) => {
                shared.jobs_run[index].fetch_add(1, Ordering::Relaxed);
                job()
            }
            None => return,
        }
    }
}

impl Jobs {
    /// Worker threads + the main thread.
    pub fn thread_count(&self) -> usize {
        self.shared.thread_count
    }

    /// Jobs executed by each thread (index 0 = main) since the pool started.
    pub fn jobs_run(&self) -> Vec<u64> {
        self.shared.jobs_run.iter().map(|c| c.load(Ordering::Relaxed)).collect()
    }

    /// `scope`: tag of the scope the job belongs to, or `None` for a background job.
    fn push(&self, job: Job, scope: Option<usize>) {
        let mut q = self.shared.queues.lock().unwrap();
        match scope {
            None => q.background.push_back(job),
            Some(tag) => {
                q.frame.push_back((tag, job));
                self.shared.frame_queued.fetch_add(1, Ordering::Relaxed);
            }
        }
        drop(q);
        self.shared.wake.notify_one();
    }

    fn try_run_one(&self, help: Help) -> bool {
        if matches!(help, Help::Scope(_)) && self.shared.frame_queued.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let job = {
            let mut q = self.shared.queues.lock().unwrap();
            let frame = match help {
                // The queue holds a handful of jobs (par_for queues one per thread), so a scan is cheap.
                Help::Scope(tag) => q.frame.iter().position(|(t, _)| *t == tag).and_then(|i| q.frame.remove(i)),
                Help::Any => q.frame.pop_front(),
            };
            match frame {
                Some((_, job)) => {
                    self.shared.frame_queued.fetch_sub(1, Ordering::Relaxed);
                    Some(job)
                }
                None if matches!(help, Help::Any) => q.background.pop_front(),
                None => None,
            }
        };
        match job {
            Some(job) => {
                if let Some(c) = self.shared.jobs_run.get(thread_index()) {
                    c.fetch_add(1, Ordering::Relaxed);
                }
                job();
                true
            }
            None => false,
        }
    }

    /// Runs queued jobs allowed by `help` until `pending` returns false. Never sleeps on a lock,
    /// so the waiting thread keeps contributing instead of idling.
    fn help_while(&self, help: Help, pending: impl Fn() -> bool) {
        if !pending() {
            return;
        }
        // Jobs run while waiting become children of this scope, so its self time is pure idle.
        let _p = profile::scope(profile::WAIT);
        let mut idle_spins = 0u32;
        while pending() {
            if self.try_run_one(help) {
                idle_spins = 0;
            } else if idle_spins < 64 {
                idle_spins += 1;
                std::hint::spin_loop();
            } else {
                thread::yield_now();
            }
        }
    }

    /// Structured parallelism: jobs spawned on the scope may borrow anything that outlives the
    /// call, because `scope` does not return until all of them have finished. A panic in any job
    /// is re-raised here after the others complete.
    pub fn scope<'env, F, R>(&self, f: F) -> R
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R,
    {
        let scope = Scope {
            jobs: self.clone(),
            pending: AtomicUsize::new(0),
            panic: Mutex::new(None),
            _scope: PhantomData,
            _env: PhantomData,
        };
        let result = panic::catch_unwind(AssertUnwindSafe(|| f(&scope)));
        self.help_while(Help::Scope(scope.tag()), || scope.pending.load(Ordering::Acquire) != 0);
        if let Some(payload) = scope.panic.lock().unwrap().take() {
            panic::resume_unwind(payload);
        }
        match result {
            Ok(r) => r,
            Err(payload) => panic::resume_unwind(payload),
        }
    }

    /// Calls `f` once per batch of `batch` consecutive indices in `0..len` (the last batch may be
    /// shorter). Batches are claimed from an atomic counter by the caller and up to
    /// `thread_count - 1` helpers, so load balances itself and only a handful of jobs are queued.
    pub fn par_for<F>(&self, len: usize, batch: usize, f: F)
    where
        F: Fn(Range<usize>) + Sync,
    {
        if len == 0 {
            return;
        }
        let batch = batch.max(1);
        let batches = len.div_ceil(batch);
        let next = AtomicUsize::new(0);
        let work = || {
            loop {
                let b = next.fetch_add(1, Ordering::Relaxed);
                if b >= batches {
                    break;
                }
                let start = b * batch;
                f(start..(start + batch).min(len));
            }
        };
        let helpers = (self.thread_count() - 1).min(batches - 1);
        if helpers == 0 {
            work();
            return;
        }
        self.scope(|s| {
            for _ in 0..helpers {
                s.spawn(work);
            }
            work();
        });
    }

    /// `par_for` over one or more slices of equal length, handing each batch the matching
    /// sub-slices: `jobs.par_zip((&mut pos, &vel), 256, |start, (pos, vel)| ...)`.
    /// `start` is the index of the batch's first element in the full slices.
    pub fn par_zip<D, F>(&self, data: D, batch: usize, f: F)
    where
        D: IntoParData,
        F: Fn(usize, <D::Data as ParData>::Chunk) + Sync,
    {
        let data = data.into_par_data();
        self.par_for(data.par_len(), batch, |range| {
            let start = range.start;
            // SAFETY: par_for hands out each range exactly once and ranges never overlap.
            f(start, unsafe { data.chunk(range) })
        });
    }

    /// Detached job on the background queue. The result is collected with `JobHandle::join`.
    pub fn spawn<T, F>(&self, f: F) -> JobHandle<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.spawn_named("job", f)
    }

    /// `spawn`, with the name its job gets in the profile.
    pub fn spawn_named<T, F>(&self, name: &'static str, f: F) -> JobHandle<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let state = Arc::new(HandleState { done: AtomicBool::new(false), result: Mutex::new(None), flow: AtomicU64::new(0) });
        let job_state = state.clone();
        let origin = Origin::here();
        self.push(
            Box::new(move || {
                let prof = origin.start(name);
                let result = panic::catch_unwind(AssertUnwindSafe(f));
                *job_state.result.lock().unwrap() = Some(result);
                // Arrow from the end of the job to whoever joins it.
                job_state.flow.store(profile::flow_out(), Ordering::Relaxed);
                drop(prof);
                job_state.done.store(true, Ordering::Release);
            }),
            None,
        );
        JobHandle { state, jobs: self.clone() }
    }
}

pub struct Scope<'scope, 'env: 'scope> {
    jobs: Jobs,
    pending: AtomicUsize,
    panic: Mutex<Option<Box<dyn Any + Send>>>,
    _scope: PhantomData<&'scope mut &'scope ()>,
    _env: PhantomData<&'env mut &'env ()>,
}

impl<'scope, 'env> Scope<'scope, 'env> {
    /// Identifies this scope's jobs in the queue: its address, unique while it is alive (and it
    /// outlives all of its jobs).
    fn tag(&self) -> usize {
        self as *const Self as usize
    }

    pub fn spawn<F>(&'scope self, f: F)
    where
        F: FnOnce() + Send + 'scope,
    {
        self.pending.fetch_add(1, Ordering::Relaxed);
        let origin = Origin::here();
        let label = profile::current_label();
        let job: Box<dyn FnOnce() + Send + 'scope> = Box::new(move || {
            let prof = origin.start(label);
            if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(f)) {
                self.panic.lock().unwrap().get_or_insert(payload);
            }
            drop(prof);
            // Last touch of `self`: after this the scope owner may return and free it.
            self.pending.fetch_sub(1, Ordering::Release);
        });
        // SAFETY: `Jobs::scope` blocks until `pending` is zero, so everything the job borrows
        // (bounded by 'scope) outlives its execution. Only the lifetime is erased.
        let job: Job = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + 'scope>, Job>(job) };
        self.jobs.push(job, Some(self.tag()));
    }
}

struct HandleState<T> {
    done: AtomicBool,
    result: Mutex<Option<thread::Result<T>>>,
    /// Profile arrow from the end of the job to the thread that joins it.
    flow: AtomicU64,
}

/// What a job remembers about where it was spawned, for the profile: the frame it works for,
/// when it was queued and the arrow from its spawner.
struct Origin {
    frame: u32,
    queued_at: u64,
    flow: u64,
}

impl Origin {
    fn here() -> Origin {
        if !profile::enabled() {
            return Origin { frame: profile::NO_FRAME, queued_at: 0, flow: 0 };
        }
        Origin { frame: profile::current_frame(), queued_at: profile::now_ns(), flow: profile::flow_out() }
    }

    /// Opens the job's scope on the thread that runs it.
    fn start(&self, name: &'static str) -> profile::Scope {
        let queued = if self.queued_at > 0 { profile::now_ns().saturating_sub(self.queued_at) } else { 0 };
        let scope = profile::job_scope(name, self.frame, queued);
        profile::flow_in(self.flow);
        scope
    }
}

pub struct JobHandle<T> {
    state: Arc<HandleState<T>>,
    jobs: Jobs,
}

impl<T> JobHandle<T> {
    pub fn is_done(&self) -> bool {
        self.state.done.load(Ordering::Acquire)
    }

    /// Waits for the job, running other jobs (including background ones) meanwhile.
    pub fn join(self) -> T {
        self.jobs.help_while(Help::Any, || !self.state.done.load(Ordering::Acquire));
        profile::flow_in(self.state.flow.load(Ordering::Relaxed));
        match self.state.result.lock().unwrap().take().expect("job result taken twice") {
            Ok(v) => v,
            Err(payload) => panic::resume_unwind(payload),
        }
    }
}

/// Data that can be split into disjoint index ranges and processed from several threads.
///
/// # Safety
/// Implementations must return non-overlapping data for non-overlapping ranges.
pub unsafe trait ParData: Sync {
    type Chunk;
    fn par_len(&self) -> usize;
    /// # Safety
    /// Ranges used concurrently must not overlap and must lie within `0..par_len()`.
    unsafe fn chunk(&self, range: Range<usize>) -> Self::Chunk;
}

pub trait IntoParData {
    type Data: ParData;
    fn into_par_data(self) -> Self::Data;
}

pub struct ParMut<'a, T> {
    ptr: *mut T,
    len: usize,
    _marker: PhantomData<&'a mut [T]>,
}

// SAFETY: chunks are disjoint `&mut [T]`, which may be sent to other threads when `T: Send`.
unsafe impl<T: Send> Sync for ParMut<'_, T> {}

unsafe impl<'a, T: Send> ParData for ParMut<'a, T> {
    type Chunk = &'a mut [T];
    fn par_len(&self) -> usize {
        self.len
    }
    unsafe fn chunk(&self, range: Range<usize>) -> &'a mut [T] {
        debug_assert!(range.start <= range.end && range.end <= self.len);
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(range.start), range.len()) }
    }
}

pub struct ParRef<'a, T>(&'a [T]);

unsafe impl<'a, T: Sync> ParData for ParRef<'a, T> {
    type Chunk = &'a [T];
    fn par_len(&self) -> usize {
        self.0.len()
    }
    unsafe fn chunk(&self, range: Range<usize>) -> &'a [T] {
        &self.0[range]
    }
}

impl<'a, T: Send> IntoParData for &'a mut [T] {
    type Data = ParMut<'a, T>;
    fn into_par_data(self) -> ParMut<'a, T> {
        ParMut { ptr: self.as_mut_ptr(), len: self.len(), _marker: PhantomData }
    }
}

impl<'a, T: Send> IntoParData for &'a mut Vec<T> {
    type Data = ParMut<'a, T>;
    fn into_par_data(self) -> ParMut<'a, T> {
        self.as_mut_slice().into_par_data()
    }
}

impl<'a, T: Sync> IntoParData for &'a [T] {
    type Data = ParRef<'a, T>;
    fn into_par_data(self) -> ParRef<'a, T> {
        ParRef(self)
    }
}

impl<'a, T: Sync> IntoParData for &'a Vec<T> {
    type Data = ParRef<'a, T>;
    fn into_par_data(self) -> ParRef<'a, T> {
        ParRef(self.as_slice())
    }
}

macro_rules! par_tuple {
    ($($name:ident $idx:tt),+) => {
        unsafe impl<$($name: ParData),+> ParData for ($($name,)+) {
            type Chunk = ($($name::Chunk,)+);
            fn par_len(&self) -> usize {
                self.0.par_len()
            }
            unsafe fn chunk(&self, range: Range<usize>) -> Self::Chunk {
                unsafe { ($(self.$idx.chunk(range.clone()),)+) }
            }
        }

        impl<$($name: IntoParData),+> IntoParData for ($($name,)+) {
            type Data = ($($name::Data,)+);
            fn into_par_data(self) -> Self::Data {
                let data = ($(self.$idx.into_par_data(),)+);
                let len = data.0.par_len();
                $(assert_eq!(data.$idx.par_len(), len, "par_zip: all slices must have the same length");)+
                data
            }
        }
    };
}

par_tuple!(A 0);
par_tuple!(A 0, B 1);
par_tuple!(A 0, B 1, C 2);
par_tuple!(A 0, B 1, C 2, D 3);
par_tuple!(A 0, B 1, C 2, D 3, E 4);
par_tuple!(A 0, B 1, C 2, D 3, E 4, F 5);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn par_for_visits_every_index_once() {
        for workers in [0, 1, 3, 7] {
            let jobs = JobSystem::new(workers);
            let hits: Vec<AtomicU32> = (0..10_007).map(|_| AtomicU32::new(0)).collect();
            jobs.par_for(hits.len(), 64, |r| {
                for i in r {
                    hits[i].fetch_add(1, Ordering::Relaxed);
                }
            });
            assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1));
        }
    }

    #[test]
    fn par_zip_writes_disjoint_chunks() {
        let jobs = JobSystem::new(3);
        let src: Vec<u32> = (0..5000).collect();
        let mut dst = vec![0u32; 5000];
        jobs.par_zip((&mut dst, &src), 100, |start, (d, s)| {
            assert_eq!(s[0] as usize, start);
            for i in 0..d.len() {
                d[i] = s[i] * 2;
            }
        });
        assert!(dst.iter().enumerate().all(|(i, &v)| v == i as u32 * 2));
    }

    #[test]
    fn nested_par_for_does_not_deadlock() {
        let jobs = JobSystem::new(2);
        let total = AtomicUsize::new(0);
        jobs.par_for(16, 1, |_| {
            jobs.par_for(100, 10, |r| {
                total.fetch_add(r.len(), Ordering::Relaxed);
            });
        });
        assert_eq!(total.load(Ordering::Relaxed), 1600);
    }

    #[test]
    fn a_waiting_scope_only_runs_its_own_jobs() {
        // No workers: every job runs on this thread, inside some wait.
        let jobs = JobSystem::new(0);
        let order = Mutex::new(Vec::new());
        jobs.scope(|outer| {
            outer.spawn(|| order.lock().unwrap().push("outer job"));
            // The outer job is first in the queue; the inner wait must not take it.
            jobs.scope(|inner| inner.spawn(|| order.lock().unwrap().push("inner job")));
            order.lock().unwrap().push("inner scope done");
        });
        assert_eq!(*order.lock().unwrap(), ["inner job", "inner scope done", "outer job"]);
    }

    #[test]
    fn scope_borrows_stack_data() {
        let jobs = JobSystem::new(2);
        let mut a = vec![0; 4];
        let (left, right) = a.split_at_mut(2);
        jobs.scope(|s| {
            s.spawn(|| left.fill(1));
            s.spawn(|| right.fill(2));
        });
        assert_eq!(a, [1, 1, 2, 2]);
    }

    #[test]
    fn panics_propagate_to_the_waiter() {
        let jobs = JobSystem::new(2);
        let r = panic::catch_unwind(AssertUnwindSafe(|| {
            jobs.par_for(10, 1, |r| {
                if r.start == 7 {
                    panic!("boom");
                }
            })
        }));
        assert!(r.is_err());
        // The pool keeps working afterwards.
        let n = AtomicUsize::new(0);
        jobs.par_for(10, 1, |_| {
            n.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(n.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn spawn_and_join_even_without_workers() {
        for workers in [0, 2] {
            let jobs = JobSystem::new(workers);
            let h = jobs.spawn(|| 40 + 2);
            assert_eq!(h.join(), 42);
        }
    }
}
