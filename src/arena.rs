//! Frame arena: bump allocator for data that lives for one frame (or one pass).
//!
//! `alloc_slice` takes `&self` and hands out disjoint `&mut [T]`, so several buffers can be live at
//! once; `reset` takes `&mut self`, so the borrow checker proves nothing allocated survives it.
//! Only `Copy` types are allowed: nothing is ever dropped, memory is just reused.

use std::{
    alloc::{self, Layout},
    cell::{Cell, UnsafeCell},
    ptr::NonNull,
};

const ALIGN: usize = 64;

struct Block {
    ptr: NonNull<u8>,
    cap: usize,
}

pub struct FrameArena {
    blocks: UnsafeCell<Vec<Block>>,
    offset: Cell<usize>,
}

// SAFETY: the arena owns its blocks; moving it to another thread moves that ownership.
unsafe impl Send for FrameArena {}

fn new_block(cap: usize) -> Block {
    let layout = Layout::from_size_align(cap.max(ALIGN), ALIGN).unwrap();
    // SAFETY: layout has non-zero size.
    let ptr = unsafe { alloc::alloc(layout) };
    Block { ptr: NonNull::new(ptr).unwrap_or_else(|| alloc::handle_alloc_error(layout)), cap: layout.size() }
}

impl FrameArena {
    pub fn with_capacity(bytes: usize) -> FrameArena {
        FrameArena { blocks: UnsafeCell::new(vec![new_block(bytes)]), offset: Cell::new(0) }
    }

    /// Allocates `len` copies of `value`. Grows by adding a block; existing slices never move.
    #[allow(clippy::mut_from_ref)]
    pub fn alloc_slice<T: Copy>(&self, len: usize, value: T) -> &mut [T] {
        assert!(align_of::<T>() <= ALIGN, "FrameArena supports alignment up to {ALIGN}");
        let size = len.checked_mul(size_of::<T>()).expect("FrameArena: allocation size overflow");
        if size == 0 {
            // SAFETY: zero bytes; a dangling, aligned pointer is a valid empty or ZST slice.
            return unsafe { std::slice::from_raw_parts_mut(NonNull::<T>::dangling().as_ptr(), len) };
        }
        // SAFETY: the Vec is only touched here and in reset/drop (which need &mut self). Handed-out
        // slices point into the blocks' heap memory, never into the Vec itself.
        let blocks = unsafe { &mut *self.blocks.get() };
        let mut start = self.offset.get().next_multiple_of(align_of::<T>());
        if start + size > blocks.last().unwrap().cap {
            let cap = blocks.last().unwrap().cap * 2;
            blocks.push(new_block(cap.max(size)));
            start = 0;
        }
        self.offset.set(start + size);
        let ptr = unsafe { blocks.last().unwrap().ptr.as_ptr().add(start) as *mut T };
        // SAFETY: [start, start + size) is inside the block, aligned for T, and not handed out before.
        unsafe {
            for i in 0..len {
                ptr.add(i).write(value);
            }
            std::slice::from_raw_parts_mut(ptr, len)
        }
    }

    /// Frees everything at once. If the frame overflowed into extra blocks, they are merged into
    /// one block big enough for the next frame, so steady state is a single allocation.
    pub fn reset(&mut self) {
        let blocks = self.blocks.get_mut();
        if blocks.len() > 1 {
            let total: usize = blocks.iter().map(|b| b.cap).sum();
            for b in blocks.drain(..) {
                free_block(b);
            }
            blocks.push(new_block(total));
        }
        self.offset.set(0);
    }

    /// Bytes handed out since the last reset (including space skipped in full blocks).
    pub fn used(&self) -> usize {
        // SAFETY: shared read; no &mut to the Vec is alive outside alloc_slice.
        let blocks = unsafe { &*self.blocks.get() };
        blocks[..blocks.len() - 1].iter().map(|b| b.cap).sum::<usize>() + self.offset.get()
    }

    pub fn capacity(&self) -> usize {
        // SAFETY: shared read; no &mut to the Vec is alive outside alloc_slice.
        unsafe { (*self.blocks.get()).iter().map(|b| b.cap).sum() }
    }
}

fn free_block(b: Block) {
    // SAFETY: allocated in new_block with this exact layout.
    unsafe { alloc::dealloc(b.ptr.as_ptr(), Layout::from_size_align_unchecked(b.cap, ALIGN)) }
}

impl Drop for FrameArena {
    fn drop(&mut self) {
        for b in self.blocks.get_mut().drain(..) {
            free_block(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_are_disjoint_aligned_and_survive_growth() {
        let mut arena = FrameArena::with_capacity(256);
        let a = arena.alloc_slice(10, 1u8);
        let b = arena.alloc_slice(100, 2.0f64);
        let c = arena.alloc_slice(1000, 3u32); // forces a new block
        assert_eq!(b.as_ptr() as usize % align_of::<f64>(), 0);
        assert!(a.iter().all(|&x| x == 1) && b.iter().all(|&x| x == 2.0) && c.iter().all(|&x| x == 3));
        a[0] = 9;
        assert_eq!(b[0], 2.0);
        let before = arena.capacity();
        arena.reset();
        assert!(arena.capacity() >= before);
        assert_eq!(arena.alloc_slice(4000, 0u8).len(), 4000);
    }
}
