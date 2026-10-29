//! Heap geral: o alocador global do Rust (`#[global_allocator]`), no modelo da Unreal
//! (FMalloc + LLM + Memory Insights).
//!
//! Os dados da engine e do jogo NÃO passam por aqui: eles saem das arenas de cada thread.
//! O heap geral atende o que não controlamos (std, dependências) e serve de vigia:
//!
//! - **Backend**: o heap do sistema (`std::alloc::System`: `HeapAlloc`/LFH no Windows, `malloc`
//!   da glibc no Linux),
//!   maduro e thread-safe. Fica atrás deste módulo e pode ser trocado (ex.: mimalloc) sem
//!   mudar quem usa.
//! - **Contabilidade**: bytes vivos, pico, alocações e liberações; por tag (categoria), como o
//!   LLM da Unreal. A tag vem da thread (`set_thread_tag`) e fica num header de 16 bytes antes
//!   de cada bloco, para a liberação descontar da tag certa.
//! - **Política** (`HeapPolicy`):
//!   - `Startup`: inicialização; aloca sem relatar.
//!   - `Report` (padrão depois de `end_startup`): aloca, conta e escreve no log, **uma vez por
//!     ponto de chamada**, a pilha de quem alocou. Nada fica invisível e nada trava.
//!   - `Forbid`: fatal na primeira alocação (testes, CI, caça a alocações).
//! - **Orçamento** (`set_budget`): ao passar, aviso no log (uma vez). Não é fatal.
//! - **OOM real** (o SO recusou): crash report com as estatísticas.
//! - **Validação**: o header guarda o tamanho; liberar com tamanho diferente é heap corrompido
//!   (crash report). Em builds de debug, memória nova é preenchida com `0xCD` e liberada com
//!   `0xDD` (como o poison da Unreal/MSVC), para uso de memória não iniciada ou já liberada
//!   aparecer rápido.
//!
//! Thread-safe por exigência da linguagem (qualquer thread pode alocar pela std): só atômicos,
//! sem locks. Durante um panic, sempre aloca (a std formata a mensagem antes do hook).
//!
//! Uso no binário (fora de testes, que usam o alocador do sistema direto):
//! ```ignore
//! #[cfg(not(test))]
//! #[global_allocator]
//! static ALLOCATOR: forja_mem::global::GlobalAllocator = forja_mem::global::GlobalAllocator;
//! ```

use core::alloc::{GlobalAlloc, Layout};
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::alloc::System;

use forja_core::text::TextBuf;
use forja_diag::{crash, log};
use forja_os::stack;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapPolicy {
    Startup = 0,
    Report = 1,
    Forbid = 2,
}

/// Categoria de uma alocação do heap geral (como as tags do LLM da Unreal).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocTag {
    Untagged = 0,
    Platform = 1,
    Graphics = 2,
    Audio = 3,
    ThirdParty = 4,
    Tools = 5,
}

pub const TAG_COUNT: usize = 6;

impl AllocTag {
    pub const ALL: [AllocTag; TAG_COUNT] = [
        AllocTag::Untagged,
        AllocTag::Platform,
        AllocTag::Graphics,
        AllocTag::Audio,
        AllocTag::ThirdParty,
        AllocTag::Tools,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AllocTag::Untagged => "untagged",
            AllocTag::Platform => "platform",
            AllocTag::Graphics => "graphics",
            AllocTag::Audio => "audio",
            AllocTag::ThirdParty => "third_party",
            AllocTag::Tools => "tools",
        }
    }

    fn from_u8(v: u8) -> AllocTag {
        match v {
            1 => AllocTag::Platform,
            2 => AllocTag::Graphics,
            3 => AllocTag::Audio,
            4 => AllocTag::ThirdParty,
            5 => AllocTag::Tools,
            _ => AllocTag::Untagged,
        }
    }
}

/// `impl` de trait exigida pela linguagem para o alocador global.
pub struct GlobalAllocator;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeapStats {
    pub live_bytes: usize,
    pub peak_bytes: usize,
    /// 0 = sem orçamento.
    pub budget: usize,
    pub allocations: u64,
    pub frees: u64,
    pub policy: HeapPolicy,
    /// Pontos de chamada distintos relatados no log desde `end_startup`.
    pub reported_sites: usize,
    /// Alocações cujo ponto de chamada não coube na tabela (não relatadas).
    pub untracked_reports: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TagStats {
    pub live_bytes: usize,
    pub allocations: u64,
}

const POISON: bool = cfg!(debug_assertions);
const POISON_NEW: u8 = 0xCD;
const POISON_FREED: u8 = 0xDD;

/// Header antes de cada bloco. O ponteiro devolvido fica alinhado a `max(align, 16)`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Header {
    size: usize,
    tag: u8,
}

const HEADER_SIZE: usize = 16;
const SITES_MAX: usize = 256;
const SITES_MASK: usize = SITES_MAX - 1;
const REPORT_FRAMES: usize = 12;

static POLICY: AtomicU8 = AtomicU8::new(HeapPolicy::Startup as u8);
static BUDGET: AtomicUsize = AtomicUsize::new(0);
static BUDGET_WARNED: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
static TAG_LIVE: [AtomicUsize; TAG_COUNT] = [const { AtomicUsize::new(0) }; TAG_COUNT];
static TAG_ALLOCATIONS: [AtomicU64; TAG_COUNT] = [const { AtomicU64::new(0) }; TAG_COUNT];
/// Hashes das pilhas já relatadas (0 = vazio).
static SITES: [AtomicU64; SITES_MAX] = [const { AtomicU64::new(0) }; SITES_MAX];
static SITES_REPORTED: AtomicUsize = AtomicUsize::new(0);
static UNTRACKED_REPORTS: AtomicU64 = AtomicU64::new(0);

// Os inicializadores já são `const`; o lint dá falso positivo neste clippy (1.97).
#[allow(clippy::missing_const_for_thread_local)]
mod thread_state {
    use core::cell::Cell;

    use super::AllocTag;

    thread_local! {
        pub(super) static THREAD_TAG: Cell<AllocTag> = const { Cell::new(AllocTag::Untagged) };
        /// Evita relatar de novo se o próprio relatório alocar (não deveria).
        pub(super) static REPORTING: Cell<bool> = const { Cell::new(false) };
    }
}

use thread_state::{REPORTING, THREAD_TAG};

// ---------------------------------------------------------------- API

pub fn set_policy(policy: HeapPolicy) {
    POLICY.store(policy as u8, Ordering::Release);
}

pub fn policy() -> HeapPolicy {
    match POLICY.load(Ordering::Acquire) {
        0 => HeapPolicy::Startup,
        2 => HeapPolicy::Forbid,
        _ => HeapPolicy::Report,
    }
}

/// Fim da inicialização: a partir daqui, cada ponto de chamada que alocar é relatado.
pub fn end_startup() {
    set_policy(HeapPolicy::Report);
}

/// Orçamento de bytes vivos no heap geral (0 = sem orçamento). Passar dele avisa no log uma vez.
pub fn set_budget(bytes: usize) {
    BUDGET.store(bytes, Ordering::Relaxed);
    BUDGET_WARNED.store(false, Ordering::Relaxed);
}

/// Tag das próximas alocações desta thread. Devolve a anterior, para restaurar depois.
pub fn set_thread_tag(tag: AllocTag) -> AllocTag {
    THREAD_TAG.try_with(|t| t.replace(tag)).unwrap_or(AllocTag::Untagged)
}

pub fn stats() -> HeapStats {
    HeapStats {
        live_bytes: LIVE.load(Ordering::Relaxed),
        peak_bytes: PEAK.load(Ordering::Relaxed),
        budget: BUDGET.load(Ordering::Relaxed),
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        frees: FREES.load(Ordering::Relaxed),
        policy: policy(),
        reported_sites: SITES_REPORTED.load(Ordering::Relaxed),
        untracked_reports: UNTRACKED_REPORTS.load(Ordering::Relaxed),
    }
}

pub fn tag_stats(tag: AllocTag) -> TagStats {
    let i = tag as usize;
    TagStats {
        live_bytes: TAG_LIVE.get(i).map_or(0, |a| a.load(Ordering::Relaxed)),
        allocations: TAG_ALLOCATIONS.get(i).map_or(0, |a| a.load(Ordering::Relaxed)),
    }
}

// ---------------------------------------------------------------- alocador

// SAFETY: devolve blocos do heap do sistema com o alinhamento pedido; o header fica antes do
// ponteiro devolvido, dentro do mesmo bloco, e a liberação recalcula o bloco original.
unsafe impl GlobalAlloc for GlobalAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: repassa o contrato de `GlobalAlloc::alloc`.
        unsafe { heap_alloc(layout, false) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: repassa o contrato de `GlobalAlloc::alloc_zeroed`.
        unsafe { heap_alloc(layout, true) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: repassa o contrato de `GlobalAlloc::dealloc`.
        unsafe { heap_free(ptr, layout) }
    }
}

/// Deslocamento do ponteiro devolvido em relação ao bloco do sistema: cabe o header e mantém
/// o alinhamento pedido.
fn header_offset(align: usize) -> usize {
    align.max(HEADER_SIZE)
}

fn current_tag() -> AllocTag {
    THREAD_TAG.try_with(|t| t.get()).unwrap_or(AllocTag::Untagged)
}

/// # Safety
/// Mesmo contrato de `GlobalAlloc::alloc` (`layout` com tamanho maior que zero).
unsafe fn heap_alloc(layout: Layout, zeroed: bool) -> *mut u8 {
    let policy = policy();
    if policy != HeapPolicy::Startup && !std::thread::panicking() {
        if policy == HeapPolicy::Forbid {
            crash::fatal(
                "alocação com o heap geral proibido",
                format_args!(
                    "pedido de {} bytes (alinhamento {}) com HeapPolicy::Forbid. \
                     A pilha abaixo mostra quem alocou (Vec, String, Box, format!...).",
                    layout.size(),
                    layout.align()
                ),
            );
        }
        report_site(layout.size());
    }

    let offset = header_offset(layout.align());
    let Some(real) = layout.size().checked_add(offset).and_then(|t| Layout::from_size_align(t, offset).ok())
    else {
        heap_oom(layout)
    };
    // SAFETY: `real` tem tamanho > 0 (inclui o header).
    let base = unsafe { if zeroed { System.alloc_zeroed(real) } else { System.alloc(real) } };
    if base.is_null() {
        heap_oom(layout);
    }
    let tag = current_tag();
    let user = base.wrapping_add(offset);
    // SAFETY: `user - 16` está dentro do bloco (offset >= 16) e alinhado a 16; `user` tem
    // `layout.size()` bytes válidos.
    unsafe {
        user.wrapping_sub(HEADER_SIZE).cast::<Header>().write(Header { size: layout.size(), tag: tag as u8 });
        if POISON && !zeroed {
            core::ptr::write_bytes(user, POISON_NEW, layout.size());
        }
    }
    account_alloc(layout.size(), tag);
    user
}

/// # Safety
/// Mesmo contrato de `GlobalAlloc::dealloc`: `ptr` veio de `heap_alloc` com o mesmo `layout`.
unsafe fn heap_free(ptr: *mut u8, layout: Layout) {
    // SAFETY: `ptr` veio de `heap_alloc`, que escreveu o header logo antes dele.
    let header = unsafe { ptr.wrapping_sub(HEADER_SIZE).cast::<Header>().read() };
    if header.size != layout.size() {
        crash::fatal(
            "heap geral corrompido",
            format_args!(
                "liberação de {} bytes num bloco que o header diz ter {} bytes \
                 (escrita fora dos limites ou liberação dupla)",
                layout.size(),
                header.size
            ),
        );
    }
    if POISON {
        // SAFETY: o bloco ainda é nosso e tem `layout.size()` bytes.
        unsafe { core::ptr::write_bytes(ptr, POISON_FREED, layout.size()) };
    }
    account_free(layout.size(), AllocTag::from_u8(header.tag));
    let offset = header_offset(layout.align());
    let Some(real) = layout.size().checked_add(offset).and_then(|t| Layout::from_size_align(t, offset).ok())
    else {
        return;
    };
    // SAFETY: mesmo bloco e mesmo layout usados em `heap_alloc`.
    unsafe { System.dealloc(ptr.wrapping_sub(offset), real) };
}

fn heap_oom(layout: Layout) -> ! {
    let s = stats();
    crash::fatal(
        "heap geral sem memória",
        format_args!(
            "o sistema recusou {} bytes (alinhamento {}); vivos no heap geral: {} bytes (pico {})",
            layout.size(),
            layout.align(),
            s.live_bytes,
            s.peak_bytes
        ),
    )
}

fn account_alloc(size: usize, tag: AllocTag) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed).wrapping_add(size);
    PEAK.fetch_max(live, Ordering::Relaxed);
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    if let Some(a) = TAG_LIVE.get(tag as usize) {
        a.fetch_add(size, Ordering::Relaxed);
    }
    if let Some(a) = TAG_ALLOCATIONS.get(tag as usize) {
        a.fetch_add(1, Ordering::Relaxed);
    }
    let budget = BUDGET.load(Ordering::Relaxed);
    if budget > 0 && live > budget && !BUDGET_WARNED.swap(true, Ordering::Relaxed) {
        log::warn(format_args!(
            "heap geral passou do orçamento: {live} bytes vivos de {budget} (tag da última alocação: {})",
            tag.name()
        ));
    }
}

fn account_free(size: usize, tag: AllocTag) {
    LIVE.fetch_sub(size, Ordering::Relaxed);
    FREES.fetch_add(1, Ordering::Relaxed);
    if let Some(a) = TAG_LIVE.get(tag as usize) {
        a.fetch_sub(size, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- relatório por ponto de chamada

fn stack_hash(frames: &[usize]) -> u64 {
    // FNV-1a.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for f in frames {
        h ^= *f as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if h == 0 { 1 } else { h }
}

/// `true` se a pilha ainda não tinha sido relatada (e a registra).
fn site_is_new(hash: u64) -> bool {
    let start = (hash as usize) & SITES_MASK;
    for i in 0..SITES_MAX {
        let Some(slot) = SITES.get(start.wrapping_add(i) & SITES_MASK) else {
            return false;
        };
        match slot.compare_exchange(0, hash, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                SITES_REPORTED.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            Err(existing) if existing == hash => return false,
            Err(_) => {}
        }
    }
    UNTRACKED_REPORTS.fetch_add(1, Ordering::Relaxed);
    false
}

/// Escreve no log a pilha de quem alocou, uma vez por ponto de chamada. Não aloca.
fn report_site(size: usize) {
    let already = REPORTING.try_with(|r| r.replace(true)).unwrap_or(true);
    if already {
        return;
    }
    let mut frames = [0usize; REPORT_FRAMES];
    let n = stack::capture(2, &mut frames);
    let frames = frames.get(..n).unwrap_or(&[]);
    if site_is_new(stack_hash(frames)) {
        let mut buf = [0u8; 768];
        let mut t = TextBuf::new(&mut buf);
        for (i, f) in frames.iter().enumerate() {
            if i > 0 {
                let _ = t.write_str(" <- ");
            }
            stack::write_module_offset(&mut t, *f);
        }
        log::warn(format_args!(
            "heap geral: alocação de {size} bytes depois da inicialização (tag {}); pilha: {}",
            current_tag().name(),
            t.as_str()
        ));
    }
    let _ = REPORTING.try_with(|r| r.set(false));
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // testes seguem as mesmas exceções do clippy.toml
mod tests {
    use super::*;

    // Chama o alocador direto (nos testes ele não é o global). Os estáticos são do processo de
    // testes: só estes testes os usam, e só com a política `Startup` (Report/Forbid são
    // testados em `tests/crash`, em processos separados).
    #[test]
    fn aligned_counted_tagged_and_poisoned() {
        for align in [8usize, 16, 64, 4096] {
            let l = Layout::from_size_align(24, align).unwrap();
            let before = tag_stats(AllocTag::Audio).live_bytes;
            let previous = set_thread_tag(AllocTag::Audio);
            let p = unsafe { GlobalAllocator.alloc(l) };
            set_thread_tag(previous);
            assert!(!p.is_null());
            assert!((p as usize).is_multiple_of(align));
            assert_eq!(tag_stats(AllocTag::Audio).live_bytes, before + 24);
            if POISON {
                assert_eq!(unsafe { *p }, POISON_NEW);
            }
            unsafe { GlobalAllocator.dealloc(p, l) };
            assert_eq!(tag_stats(AllocTag::Audio).live_bytes, before);
        }
    }

    #[test]
    fn zeroed_allocation_is_zero() {
        let l = Layout::from_size_align(64, 8).unwrap();
        let p = unsafe { GlobalAllocator.alloc_zeroed(l) };
        assert!((0..64).all(|i| unsafe { *p.add(i) } == 0));
        unsafe { GlobalAllocator.dealloc(p, l) };
    }

    #[test]
    fn each_site_is_reported_once() {
        let h = stack_hash(&[0x1234, 0x5678]);
        assert!(site_is_new(h));
        assert!(!site_is_new(h));
        assert_ne!(stack_hash(&[0x1234, 0x5678]), stack_hash(&[0x5678, 0x1234]));
    }
}
