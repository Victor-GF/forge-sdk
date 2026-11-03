//! Pool de threads acordado por semaforo, no estilo de uma fila de trabalho.
//!
//! A cada `run`, o pool recebe `jobs` sinais em `work`; cada worker acordado
//! faz um job e devolve um sinal em `done`. Quem chamou espera todos.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use forge_os::prelude::*;

struct Shared {
    work: SemaphoreHandle,
    done: SemaphoreHandle,
    quit: AtomicBool,
    completed: AtomicU32,
}

pub struct Pool {
    shared: Box<Shared>,
    threads: Vec<ThreadHandle>,
}

fn worker(param: *mut c_void) -> u32 {
    let shared = unsafe { &*(param as *const Shared) };
    let mut jobs = 0;
    loop {
        Platform::semaphore_wait(&shared.work);
        if shared.quit.load(Ordering::Acquire) {
            return jobs;
        }
        shared.completed.fetch_add(1, Ordering::Relaxed);
        jobs += 1;
        Platform::semaphore_signal(&shared.done, 1);
    }
}

impl Pool {
    pub fn start(count: u32) -> Pool {
        let shared = Box::new(Shared {
            work: Platform::semaphore_create(0, i32::MAX as u32).expect("semaphore_create falhou"),
            done: Platform::semaphore_create(0, i32::MAX as u32).expect("semaphore_create falhou"),
            quit: AtomicBool::new(false),
            completed: AtomicU32::new(0),
        });
        let param = &*shared as *const Shared as *mut c_void;
        let threads = (0..count)
            .map(|_| Platform::thread_create(worker, param).expect("thread_create falhou"))
            .collect();
        println!("[workers] {count} threads");
        Pool { shared, threads }
    }

    pub fn len(&self) -> u32 {
        self.threads.len() as u32
    }

    /// Distribui `jobs` jobs entre os workers e bloqueia ate todos terminarem.
    pub fn run(&self, jobs: u32) {
        let before = self.shared.completed.load(Ordering::Relaxed);
        Platform::semaphore_signal(&self.shared.work, jobs);
        for _ in 0..jobs {
            Platform::semaphore_wait(&self.shared.done);
        }
        let after = self.shared.completed.load(Ordering::Relaxed);
        assert_eq!(after - before, jobs, "workers perderam jobs");
    }

    /// Acorda todos para sair, espera cada um e devolve o total de jobs.
    pub fn stop(self) -> u32 {
        self.shared.quit.store(true, Ordering::Release);
        Platform::semaphore_signal(&self.shared.work, self.len());

        let mut total = 0;
        for (i, thread) in self.threads.into_iter().enumerate() {
            let jobs = Platform::thread_join(thread);
            println!("[workers] worker {i}: {jobs} jobs");
            total += jobs;
        }
        assert_eq!(total, self.shared.completed.load(Ordering::Relaxed), "soma dos workers diverge do contador");

        let shared = *self.shared;
        Platform::semaphore_destroy(shared.work);
        Platform::semaphore_destroy(shared.done);
        total
    }
}
