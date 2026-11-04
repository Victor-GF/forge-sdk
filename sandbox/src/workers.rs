//! Pool de threads: semaforo so para acordar, atomicos para distribuir.
//!
//! As threads sao criadas uma vez em `start` e dormem em `wake` ate haver
//! trabalho. `run` publica um lote e acorda workers; cada um, e a propria
//! thread que chamou `run`, fica pegando indices do lote ate acabarem e so
//! entao volta a dormir. Quem termina o ultimo job do lote sinaliza `done`.
//!
//! Por lote, o kernel e chamado no maximo `workers + 2` vezes, nao importa
//! quantos jobs: o custo por job e um CAS e um `fetch_sub`.
//!
//! # Indices globais
//!
//! `next` e `end` so crescem, nunca voltam a zero. O lote atual ocupa os
//! indices `base..end`; `jobs[i - base]` e o job do indice `i`. Pegar um
//! indice e um CAS em `next` que so passa se `next < end`. Disso sai que:
//!
//! - Ao publicar um lote, `next == base`: o lote anterior so terminou depois
//!   de todos os seus indices serem pegos, e nenhum CAS passa de `end`.
//! - Um worker atrasado que leu o `end` de um lote antigo nao pega nada.
//! - Quem pegou um indice do lote atual le `base` e `jobs` desse lote: eles
//!   foram publicados antes de `end` (`Release`), e o lote seguinte nao pode
//!   comecar enquanto esse job nao terminar.

use core::cell::Cell;
use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use forge_os::prelude::*;

/// Uma tarefa: quem a pegar chama `func(data)`.
///
/// `data` e acessado de outra thread e precisa continuar valido ate o `run`
/// que recebeu o job retornar. Jobs do mesmo lote rodam ao mesmo tempo:
/// nenhum pode escrever no que outro le ou escreve.
pub struct Job {
    pub func: fn(data: *mut c_void),
    pub data: *mut c_void,
}

/// Estado visto pelos workers. Fica no heap, num endereco fixo, porque cada
/// worker guarda um ponteiro para ca desde `thread_create`.
struct Shared {
    /// Acorda workers. Fichas a mais sao inofensivas: o worker nao acha
    /// indice livre e volta a dormir.
    wake: SemaphoreHandle,
    /// Uma ficha por lote, de quem terminou o ultimo job.
    done: SemaphoreHandle,
    quit: AtomicBool,
    /// Lote atual e o indice global do `jobs[0]`.
    jobs: AtomicPtr<Job>,
    base: AtomicU64,
    /// Um depois do ultimo indice do lote atual. So `run` escreve.
    end: AtomicU64,
    /// Proximo indice global a ser pego.
    next: AtomicU64,
    /// Jobs do lote atual que ainda nao terminaram.
    remaining: AtomicU64,
}

/// Dono das threads e do `Shared`. Nao e `Sync`: um `run` por vez, sempre da
/// mesma thread. Um job nao pode chamar `run` no proprio pool.
pub struct Pool {
    /// Vem de `Box::into_raw`; volta a ser `Box` so no `drop`, depois do join
    /// de todos os workers.
    shared: *mut Shared,
    threads: Vec<ThreadHandle>,
    submitted: Cell<u64>,
    /// Jobs executados pela propria thread que chama `run`.
    caller_executed: Cell<u64>,
}

/// Pega e executa jobs do lote atual ate nao sobrar indice livre. Devolve
/// quantos executou. Usado pelos workers e por `run`.
fn drain(shared: &Shared) -> u32 {
    let mut executed = 0;
    loop {
        let end = shared.end.load(Ordering::Acquire);
        let mut i = shared.next.load(Ordering::Relaxed);
        loop {
            if i >= end {
                return executed;
            }
            match shared.next.compare_exchange_weak(i, i + 1, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(current) => i = current,
            }
        }

        // `i` e do lote publicado junto com `end`: `base` e `jobs` sao dele.
        let base = shared.base.load(Ordering::Relaxed);
        let jobs = shared.jobs.load(Ordering::Relaxed);
        let job = unsafe { &*jobs.add((i - base) as usize) };
        (job.func)(job.data);
        executed += 1;

        // Depois daqui ninguem toca mais no job; quem zera libera o `run`.
        if shared.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            Platform::semaphore_signal(&shared.done, 1);
        }
    }
}

/// Devolve quantos jobs este worker executou.
fn worker(param: *mut c_void) -> u32 {
    let shared = unsafe { &*(param as *const Shared) };
    let mut executed = 0;
    loop {
        Platform::semaphore_wait(&shared.wake);
        if shared.quit.load(Ordering::Acquire) {
            return executed;
        }
        executed += drain(shared);
    }
}

impl Pool {
    /// Cria `count` workers, todos dormindo ate o primeiro `run`. A thread
    /// que chama `run` tambem executa jobs: com N nucleos, `count = N - 1`.
    pub fn start(count: u32) -> Pool {
        assert!(count > 0, "pool sem workers");
        let shared = Box::into_raw(Box::new(Shared {
            wake: Platform::semaphore_create(0, i32::MAX as u32).expect("semaphore_create falhou"),
            done: Platform::semaphore_create(0, 1).expect("semaphore_create falhou"),
            quit: AtomicBool::new(false),
            jobs: AtomicPtr::new(ptr::null_mut()),
            base: AtomicU64::new(0),
            end: AtomicU64::new(0),
            next: AtomicU64::new(0),
            remaining: AtomicU64::new(0),
        }));
        let threads = (0..count)
            .map(|_| Platform::thread_create(worker, shared.cast()).expect("thread_create falhou"))
            .collect();
        println!("[workers] {count} threads + quem chama run");
        Pool { shared, threads, submitted: Cell::new(0), caller_executed: Cell::new(0) }
    }

    #[inline]
    fn shared(&self) -> &Shared {
        unsafe { &*self.shared }
    }

    pub fn len(&self) -> u32 {
        self.threads.len() as u32
    }

    /// Executa `jobs` nos workers e na thread atual, e bloqueia ate todos
    /// terminarem. Ao retornar, ninguem toca mais em `jobs` nem nos dados
    /// deles.
    pub fn run(&self, jobs: &[Job]) {
        if jobs.is_empty() {
            return;
        }
        let count = jobs.len() as u64;
        let shared = self.shared();

        // So `run` escreve `end`: o lote novo comeca onde o anterior acabou.
        let base = shared.end.load(Ordering::Relaxed);
        let end = base + count;
        shared.jobs.store(jobs.as_ptr().cast_mut(), Ordering::Relaxed);
        shared.base.store(base, Ordering::Relaxed);
        shared.remaining.store(count, Ordering::Relaxed);
        shared.end.store(end, Ordering::Release);

        // Esta thread pega um job; acorda workers para o resto.
        let helpers = (count - 1).min(self.len() as u64) as u32;
        if helpers > 0 {
            Platform::semaphore_signal(&shared.wake, helpers);
        }

        let mine = drain(shared);
        Platform::semaphore_wait(&shared.done);

        assert_eq!(shared.next.load(Ordering::Relaxed), end, "indices pegos a mais ou a menos");
        self.caller_executed.set(self.caller_executed.get() + mine as u64);
        self.submitted.set(self.submitted.get() + count);
    }

    /// Encerra os workers e devolve o total de jobs executados.
    pub fn stop(mut self) -> u64 {
        let workers = self.join_all();
        let caller = self.caller_executed.get();
        println!("[workers] quem chama run: {caller} jobs");
        let total = workers + caller;
        assert_eq!(total, self.submitted.get(), "jobs executados divergem dos enviados");
        total
    }

    /// Acorda todos para sair, espera cada um e devolve a soma dos jobs que
    /// executaram. Sem threads (ja encerrado), nao faz nada.
    fn join_all(&mut self) -> u64 {
        if self.threads.is_empty() {
            return 0;
        }
        let shared = self.shared();
        shared.quit.store(true, Ordering::Release);
        Platform::semaphore_signal(&shared.wake, self.len());

        let mut total = 0;
        for (i, thread) in self.threads.drain(..).enumerate() {
            let jobs = Platform::thread_join(thread);
            println!("[workers] worker {i}: {jobs} jobs");
            total += jobs as u64;
        }
        total
    }
}

/// Os workers guardam ponteiro para o `Shared`: so da para liberar depois do
/// join de todos. Vale com ou sem `stop`.
impl Drop for Pool {
    fn drop(&mut self) {
        self.join_all();
        let shared = unsafe { Box::from_raw(self.shared) };
        let Shared { wake, done, .. } = *shared;
        Platform::semaphore_destroy(wake);
        Platform::semaphore_destroy(done);
    }
}
