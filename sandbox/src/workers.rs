//! Pool de threads: semaforo so para acordar, atomicos para distribuir.
//!
//! As threads sao criadas uma vez em `start` e dormem em `wake` ate haver
//! trabalho. `run` publica um lote e acorda workers; cada um, e a propria
//! thread que chamou `run`, fica pegando blocos de indices do lote ate
//! acabarem e so entao volta a dormir. Quem termina o ultimo job do lote
//! sinaliza `done`.
//!
//! Por lote, o kernel e chamado no maximo `workers + 2` vezes, nao importa
//! quantos jobs. O custo de coordenacao e um CAS e um `fetch_sub` por bloco.
//!
//! # Blocos
//!
//! `next` e `remaining` sao true sharing: toda thread escreve neles, e cada
//! escrita move a linha de cache entre cores (~100 ns). Pegando um indice
//! por vez, esse custo e por job e limita a vazao do pool inteiro, qualquer
//! que seja o numero de threads.
//!
//! Por isso cada CAS pega um bloco: o que falta no lote dividido por
//! `split` (participantes x `BLOCKS_PER_PARTICIPANT`), no minimo 1. Blocos
//! grandes no comeco amortizam a linha contestada; blocos pequenos no fim
//! mantem o balanceamento, sem uma thread presa com o ultimo bloco grande
//! enquanto as outras esperam.
//!
//! # Indices globais
//!
//! `next` e `end` so crescem, nunca voltam a zero. O lote atual ocupa os
//! indices `base..end`; `jobs[i - base]` e o job do indice `i`. Pegar um
//! bloco e um CAS em `next` que nunca passa de `end`. Disso sai que:
//!
//! - Ao publicar um lote, `next == base`: o lote anterior so terminou depois
//!   de todos os seus indices serem pegos, e nenhum CAS passa de `end`.
//! - Um worker atrasado que leu o `end` de um lote antigo nao pega nada.
//! - Quem pegou um bloco do lote atual le `base` e `jobs` desse lote: eles
//!   foram publicados antes de `end` (`Release`), e o lote seguinte nao pode
//!   comecar enquanto esse bloco nao terminar.
//!
//! # Cache
//!
//! O estado compartilhado fica em tres linhas isoladas, por padrao de escrita:
//!
//! - `batch`: escrito uma vez por lote, lido a cada job. Fica em cache em
//!   todas as cores durante o lote inteiro.
//! - `next`: CAS a cada bloco pego. True sharing inevitavel; isolado para
//!   nao invalidar `batch` junto.
//! - `remaining`: `fetch_sub` a cada bloco terminado. Separado de `next`:
//!   quem esta pegando um bloco nao disputa linha com quem terminou outro.

use core::cell::Cell;
use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use forge_core::cache::CacheAligned;
use forge_os::prelude::*;

/// Em quantos blocos, no maximo, cada participante divide o que falta. Mais
/// blocos: melhor balanceamento, mais CAS em `next`.
const BLOCKS_PER_PARTICIPANT: u64 = 4;

/// Uma tarefa: quem a pegar chama `func(data)`.
///
/// `data` e acessado de outra thread e precisa continuar valido ate o `run`
/// que recebeu o job retornar. Jobs do mesmo lote rodam ao mesmo tempo:
/// nenhum pode escrever no que outro le ou escreve.
pub struct Job {
    pub func: fn(data: *mut c_void),
    pub data: *mut c_void,
}

/// O que muda so entre lotes: lido por todos, escrito por `run`.
struct Batch {
    /// Acorda workers. Fichas a mais sao inofensivas: o worker nao acha
    /// indice livre e volta a dormir.
    wake: SemaphoreHandle,
    /// Uma ficha por lote, de quem terminou o ultimo job.
    done: SemaphoreHandle,
    quit: AtomicBool,
    /// Lote atual e o indice global do `jobs[0]`.
    jobs: AtomicPtr<Job>,
    base: AtomicU64,
    /// Um depois do ultimo indice do lote atual.
    end: AtomicU64,
    /// Divisor do que falta para achar o tamanho do bloco. Fixo desde
    /// `start`: participantes (workers + quem chama `run`) x
    /// `BLOCKS_PER_PARTICIPANT`.
    split: u64,
}

/// Estado visto pelos workers. Fica no heap, num endereco fixo, porque cada
/// worker guarda um ponteiro para ca desde `thread_create`.
struct Shared {
    batch: CacheAligned<Batch>,
    /// Proximo indice global a ser pego.
    next: CacheAligned<AtomicU64>,
    /// Jobs do lote atual que ainda nao terminaram.
    remaining: CacheAligned<AtomicU64>,
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

/// Pega e executa blocos do lote atual ate nao sobrar indice livre. Devolve
/// quantos jobs executou. Usado pelos workers e por `run`.
fn drain(shared: &Shared) -> u32 {
    let batch = &shared.batch;
    let mut executed = 0;
    loop {
        // Pega `first..first + size`. O tamanho e recalculado a cada
        // tentativa: depende de quanto falta, que muda se o CAS perder.
        let end = batch.end.load(Ordering::Acquire);
        let mut first = shared.next.load(Ordering::Relaxed);
        let size = loop {
            if first >= end {
                return executed;
            }
            let size = ((end - first) / batch.split).max(1);
            match shared.next.compare_exchange_weak(
                first,
                first + size,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break size,
                Err(current) => first = current,
            }
        };

        // O bloco e do lote publicado junto com `end`: `base` e `jobs` sao
        // dele.
        let base = batch.base.load(Ordering::Relaxed);
        let jobs = unsafe { batch.jobs.load(Ordering::Relaxed).add((first - base) as usize) };
        for k in 0..size as usize {
            let job = unsafe { &*jobs.add(k) };
            (job.func)(job.data);
        }
        executed += size as u32;

        // Depois daqui ninguem toca mais no bloco; quem zera libera o `run`.
        if shared.remaining.fetch_sub(size, Ordering::AcqRel) == size {
            Platform::semaphore_signal(&batch.done, 1);
        }
    }
}

/// Devolve quantos jobs este worker executou.
fn worker(param: *mut c_void) -> u32 {
    let shared = unsafe { &*(param as *const Shared) };
    let mut executed = 0;
    loop {
        Platform::semaphore_wait(&shared.batch.wake);
        if shared.batch.quit.load(Ordering::Acquire) {
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
            batch: CacheAligned::new(Batch {
                wake: Platform::semaphore_create(0, i32::MAX as u32)
                    .expect("semaphore_create falhou"),
                done: Platform::semaphore_create(0, 1).expect("semaphore_create falhou"),
                quit: AtomicBool::new(false),
                jobs: AtomicPtr::new(ptr::null_mut()),
                base: AtomicU64::new(0),
                end: AtomicU64::new(0),
                split: (count as u64 + 1) * BLOCKS_PER_PARTICIPANT,
            }),
            next: CacheAligned::new(AtomicU64::new(0)),
            remaining: CacheAligned::new(AtomicU64::new(0)),
        }));
        let threads = (0..count)
            .map(|index| {
                let name = format!("forge worker {index}");
                let options = ThreadOptions { name: &name, ..ThreadOptions::default() };
                Platform::thread_create(worker, shared.cast(), &options).expect("thread_create falhou")
            })
            .collect();
        forge_log!("[workers] {count} threads + quem chama run");
        Pool {
            shared,
            threads,
            submitted: Cell::new(0),
            caller_executed: Cell::new(0),
        }
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
        let batch = &shared.batch;

        // So `run` escreve `end`: o lote novo comeca onde o anterior acabou.
        let base = batch.end.load(Ordering::Relaxed);
        let end = base + count;
        batch
            .jobs
            .store(jobs.as_ptr().cast_mut(), Ordering::Relaxed);
        batch.base.store(base, Ordering::Relaxed);
        shared.remaining.store(count, Ordering::Relaxed);
        batch.end.store(end, Ordering::Release);

        // Esta thread pega um job; acorda workers para o resto.
        let helpers = (count - 1).min(self.len() as u64) as u32;
        if helpers > 0 {
            Platform::semaphore_signal(&batch.wake, helpers);
        }

        let mine = drain(shared);
        Platform::semaphore_wait(&batch.done);

        assert_eq!(
            shared.next.load(Ordering::Relaxed),
            end,
            "indices pegos a mais ou a menos"
        );
        self.caller_executed
            .set(self.caller_executed.get() + mine as u64);
        self.submitted.set(self.submitted.get() + count);
    }

    /// Encerra os workers e devolve o total de jobs executados.
    pub fn stop(mut self) -> u64 {
        let workers = self.join_all();
        let caller = self.caller_executed.get();
        forge_log!("[workers] quem chama run: {caller} jobs");
        let total = workers + caller;
        assert_eq!(
            total,
            self.submitted.get(),
            "jobs executados divergem dos enviados"
        );
        total
    }

    /// Acorda todos para sair, espera cada um e devolve a soma dos jobs que
    /// executaram. Sem threads (ja encerrado), nao faz nada.
    fn join_all(&mut self) -> u64 {
        if self.threads.is_empty() {
            return 0;
        }
        let batch = &self.shared().batch;
        batch.quit.store(true, Ordering::Release);
        Platform::semaphore_signal(&batch.wake, self.len());

        let mut total = 0;
        for (i, thread) in self.threads.drain(..).enumerate() {
            let jobs = Platform::thread_join(thread);
            forge_log!("[workers] worker {i}: {jobs} jobs");
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
        let Batch { wake, done, .. } = shared.batch.into_inner();
        Platform::semaphore_destroy(wake);
        Platform::semaphore_destroy(done);
    }
}
