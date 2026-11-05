//! Exercita paginas e heap uma vez, na inicializacao.

use core::{ptr, slice};
use forge_os::prelude::*;

pub fn run() {
    let page = Platform::page_info();
    log!("[memory] page={} granularity={}", page.size, page.granularity);

    pages(page);
    heap();
    failures();

    log!("[memory] ok");
}

fn pages(page: PageInfo) {
    let reserved = 16 * page.size;
    let committed = 4 * page.size;

    let base = Platform::pages_reserve(reserved);
    assert!(!base.is_null(), "pages_reserve falhou");
    unsafe {
        assert!(Platform::pages_commit(base, committed), "pages_commit falhou");
        let bytes = slice::from_raw_parts_mut(base, committed);
        assert!(bytes.iter().all(|&b| b == 0), "commit nao veio zerado");
        bytes.fill(0xAB);
        assert!(bytes.iter().all(|&b| b == 0xAB));

        Platform::pages_decommit(base, committed);
        assert!(Platform::pages_commit(base, committed), "recommit falhou");
        assert!(slice::from_raw_parts(base, committed).iter().all(|&b| b == 0), "recommit nao veio zerado");
        Platform::pages_release(base);
    }

    let size = 1 << 20;
    let block = Platform::pages_alloc(size);
    assert!(!block.is_null(), "pages_alloc falhou");
    assert_eq!(block as usize % page.granularity, 0, "pages_alloc fora da granularidade");
    unsafe {
        let bytes = slice::from_raw_parts_mut(block, size);
        assert!(bytes.iter().all(|&b| b == 0), "pages_alloc nao veio zerado");
        bytes.fill(0xCD);
        Platform::pages_release(block);
    }
}

fn heap() {
    let zeroed = Platform::heap_alloc_zeroed(64);
    assert!(!zeroed.is_null(), "heap_alloc_zeroed falhou");
    assert_eq!(zeroed as usize % 16, 0, "heap fora do alinhamento de 16");

    unsafe {
        assert!(slice::from_raw_parts(zeroed, 64).iter().all(|&b| b == 0), "heap_alloc_zeroed nao veio zerado");
        Platform::heap_free(zeroed);

        let small = Platform::heap_alloc(32);
        assert!(!small.is_null(), "heap_alloc falhou");
        for i in 0..32 {
            small.add(i).write(i as u8);
        }

        let grown = Platform::heap_realloc(small, 4096);
        assert!(!grown.is_null(), "heap_realloc falhou");
        for i in 0..32 {
            assert_eq!(grown.add(i).read(), i as u8, "heap_realloc perdeu o conteudo");
        }

        Platform::heap_free(grown);
        Platform::heap_free(ptr::null_mut());
    }
}

/// Pedidos impossiveis precisam voltar nulo, nunca um ponteiro invalido.
/// No realloc que falha, o bloco original tem que continuar intacto.
fn failures() {
    let impossible = usize::MAX - 1024;

    assert!(Platform::heap_alloc(impossible).is_null(), "heap_alloc impossivel nao falhou");
    assert!(Platform::heap_alloc_zeroed(impossible).is_null(), "heap_alloc_zeroed impossivel nao falhou");
    assert!(Platform::pages_alloc(impossible).is_null(), "pages_alloc impossivel nao falhou");
    assert!(Platform::pages_reserve(impossible).is_null(), "pages_reserve impossivel nao falhou");

    let block = Platform::heap_alloc(16);
    assert!(!block.is_null(), "heap_alloc falhou");
    unsafe {
        block.write_bytes(0x5A, 16);
        assert!(Platform::heap_realloc(block, impossible).is_null(), "heap_realloc impossivel nao falhou");
        assert!(slice::from_raw_parts(block, 16).iter().all(|&b| b == 0x5A), "realloc que falhou estragou o bloco");
        Platform::heap_free(block);
    }

    log!("[memory] falhas viram nulo");
}
