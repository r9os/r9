use core::ptr::addr_of;

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
use crate::vm::Entry;
use crate::vm::PageTable;
use crate::vm::PhysPage4K;
use crate::vm::VirtPage4K;
use port::bitmapalloc::BitmapPageAlloc;
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
        panic!("Couldn't mark early pages free: range: {} err: {:?}", early_pages_range, err);
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
    // TODO: Fix this, as it wastes nearly 2MiB
    page_alloc.mark_allocated(&kmem::early_pages_range())
}

/// Try to allocate a physical page.  Note that this is NOT mapped.
pub fn allocate_physpage() -> Result<&'static mut PhysPage4K, PageAllocError> {
    let node = LockNode::new();
    let mut lock = PAGE_ALLOC.lock(&node);
    let page_alloc = &mut *lock;

    match page_alloc.allocate() {
        Ok(page_pa) => {
            println!("pagealloc::allocate pa:{:?}", page_pa);
            Ok(unsafe { &mut *physaddr_as_ptr_mut::<PhysPage4K>(page_pa) })
        }
        Err(err) => Err(err),
    }
}

/// Try to allocate a physical page and map it into virtual memory.
pub fn allocate_virtpage(
    kpage_table: &mut PageTable,
) -> Result<&'static mut VirtPage4K, PageAllocError> {
    let physpage = allocate_physpage()?;
    let pagepa = addr_of!(physpage) as u64;
    let range = PhysRange::with_end(pagepa, pagepa + PAGE_SIZE_4K as u64);
    // TODO making a bit of an assumption here...
    let entry = Entry::rw_user_text();
    if let Ok(page_va) =
        kpage_table.map_phys_range(&range, 4096, entry, crate::vm::PageSize::Page4K)
    {
        println!("pagealloc::allocate va:{:#x}", page_va.0);
        let virtpage = page_va.0 as *mut VirtPage4K;
        Ok(unsafe { &mut *virtpage })
    } else {
        println!("pagealloc::allocate unable to map");
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
