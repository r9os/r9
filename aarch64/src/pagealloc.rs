/// aarch64 uses of the page allocator, which is port::pagealloc: zeroed pages,
/// for page tables.
use crate::kmap::kmap;
use port::mem::PAGE_SIZE_4K;
use port::mem::PhysAddr;
use port::pagealloc::{PageAllocError, allocate_physpage};

/// Try to allocate a physical page, zeroed through kmap.  Note that this is NOT
/// mapped.
pub fn allocate_zeroed_physpage() -> Result<PhysAddr, PageAllocError> {
    let page_pa = allocate_physpage()?;
    let k = kmap(page_pa);
    // SAFETY: kmaps the whole of the newly allocated page, which nothing else
    // refers to yet.
    unsafe { core::ptr::write_bytes(k.va() as *mut u8, 0, PAGE_SIZE_4K) };
    Ok(page_pa)
}
