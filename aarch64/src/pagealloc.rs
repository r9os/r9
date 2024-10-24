/// This module acts as an interface between the portable allocator and the
/// arch-specific use of it.
///
/// The page allocator is constructed and finalised in a number of phases:
/// 1. `init_page_allocator` to create a fixed size allocator assuming everything
///    is in use except a small number of statically defined pages available for
///    setting up the initial page tables.
/// 2. `free_unused_ranges` to mark available ranges as the inverse of the
///    physical memory map within the bounds of the available memory.
use crate::kmem;
use crate::kmem::physaddr_as_ptr_mut;
use crate::vm::Page4K;
use port::bitmapalloc::BitmapPageAlloc;
use port::bitmapalloc::BitmapPageAllocError;
use port::mem::PhysRange;
use port::{
    mcslock::{Lock, LockNode},
    mem::PAGE_SIZE_4K,
};

use core::fmt;

#[cfg(not(test))]
use port::println;

/// Set up bitmap page allocator assuming everything is allocated.
static PAGE_ALLOC: Lock<BitmapPageAlloc<32, PAGE_SIZE_4K>> = Lock::new(
    "page_alloc",
    const { BitmapPageAlloc::<32, PAGE_SIZE_4K>::new_all_allocated(PAGE_SIZE_4K) },
);

/// The bitmap allocator has all pages marked as allocated initially.  We'll
/// add some pages (mark free) to allow us to set up the page tables and build
/// a memory map.  Once the memory map has been build, we can mark all the unused
/// space as available.  This allows us to use only one page allocator throughout.
pub fn init_page_allocator() {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;

    let early_pages_range = kmem::early_pages_range();
    if let Err(err) = page_alloc.mark_free(&early_pages_range) {
        panic!("Couldn't mark early pages free: range: {} err: {:?}", early_pages_range, err);
    }
}

/// Free unused pages in mem that aren't covered by the memory map.  Assumes
/// that custom_map is sorted.
pub fn free_unused_ranges<'a>(
    available_mem: &PhysRange,
    used_ranges: impl Iterator<Item = &'a PhysRange>,
) -> Result<(), BitmapPageAllocError> {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;

    page_alloc.free_unused_ranges(available_mem, used_ranges)
}

/// Try to allocate a page
pub fn allocate() -> Result<&'static mut Page4K, BitmapPageAllocError> {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;

    println!("pagealloc::allocate");

    match page_alloc.allocate() {
        Ok(page_pa) => Ok(unsafe { &mut *physaddr_as_ptr_mut::<Page4K>(page_pa) }),
        Err(err) => Err(err),
    }
}

/// Return a tuple of (bytes used, total bytes available) based on the page allocator.
pub fn usage_bytes() -> (usize, usize) {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;
    page_alloc.usage_bytes()
}
