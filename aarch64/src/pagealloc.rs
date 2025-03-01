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
use crate::vm::Entry;
use crate::vm::RootPageTable;
use crate::vm::RootPageTableType;
use crate::vm::VaMapping;
use crate::vm::VirtPage4K;
use port::bitmapalloc::BitmapPageAlloc;
use port::mem::PhysAddr;
use port::mem::PhysRange;
use port::pagealloc::PageAllocError;
use port::{
    mcslock::{Lock, LockNode},
    mem::PAGE_SIZE_4K,
};

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
        panic!(
            "error:pagealloc:init_page_allocator:couldn't mark early pages free: range: {} err: {:?}",
            early_pages_range, err
        );
    }
}

/// Free unused pages in mem that aren't covered by the memory map.  Assumes
/// that custom_map is sorted.
pub fn free_unused_ranges<'a>(
    available_mem: &PhysRange,
    used_ranges: impl Iterator<Item = &'a PhysRange>,
) -> Result<(), PageAllocError> {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;

    page_alloc.free_unused_ranges(available_mem, used_ranges)?;

    // Mark all the early pages as used.  The early pages are all mapped, but we want to
    // assume that past this point all pages are unmapped.  The mapping can then be always
    // done after allocating a page.  The downside is that we lose access to the unallocated
    // early pages.
    page_alloc.mark_allocated(&kmem::early_pages_range())
}

/// Try to allocate a physical page.  Note that this is NOT mapped.
pub fn allocate_physpage() -> Result<PhysAddr, PageAllocError> {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;

    match page_alloc.allocate() {
        Ok(page_pa) => {
            println!("pagealloc:allocate_physpage pa:{:?}", page_pa);
            Ok(page_pa)
        }
        Err(err) => {
            println!("error:pagealloc:allocate_physpage:failed to allocate: {:?}", err);
            Err(err)
        }
    }
}

/// Try to allocate a physical page and map it into virtual memory at va.
pub fn allocate_virtpage(
    page_table: &mut RootPageTable,
    debug_name: &str,
    entry: Entry,
    va: VaMapping,
    pgtype: RootPageTableType,
) -> Result<&'static mut VirtPage4K, PageAllocError> {
    println!("pagealloc:allocate_virtpage:start");
    let page_pa = allocate_physpage()?;
    let range = PhysRange::with_pa_len(page_pa, PAGE_SIZE_4K);
    println!("pagealloc:allocate_virtpage:try map");
    if let Ok(page_va) = page_table.map_phys_range(
        debug_name,
        &range,
        va,
        entry,
        crate::vm::PageSize::Page4K,
        pgtype,
    ) {
        println!("pagealloc:allocate_virtpage:va:{:#x} -> physpage:{:?}", page_va.0, page_pa);
        let virtpage = page_va.0 as *mut VirtPage4K;
        Ok(unsafe { &mut *virtpage })
    } else {
        println!("error:pagealloc:allocate_virtpage:unable to map");
        Err(PageAllocError::UnableToMap)
    }
}

/// Return a tuple of (bytes used, total bytes available) based on the page allocator.
pub fn usage_bytes() -> (usize, usize) {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;
    page_alloc.usage_bytes()
}
