/// aarch64 uses of the page allocator, which is port::pagealloc: zeroed pages,
/// and pages mapped into the kernel or a user address space.
use crate::kmap::kmap;
use crate::vm;
use crate::vm::Entry;
use crate::vm::PageSize;
use crate::vm::PhysPageAllocator;
use crate::vm::RootPageTableType;
use crate::vm::VaMapping;
use crate::vm::VmTraitImpl;
use port::mem::PAGE_SIZE_4K;
use port::mem::PhysAddr;
use port::mem::PhysRange;
use port::mem::VirtRange;
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

/// Allocate, zero and map a writable data page for each 4KiB page in `range`,
/// in the active user address space.  The pages are zeroed before they're
/// mapped.  If it fails partway, the pages it mapped stay in the address space,
/// and are freed when the address space is released.
pub fn map_zeroed_user_pages(debug_name: &str, range: VirtRange) -> Result<(), PageAllocError> {
    assert!(
        range.start.is_multiple_of(PAGE_SIZE_4K) && range.end.is_multiple_of(PAGE_SIZE_4K),
        "map_zeroed_user_pages: range {range} isn't page aligned"
    );

    for va in (range.start..range.end).step_by(PAGE_SIZE_4K) {
        let page_pa = allocate_zeroed_physpage()?;
        let mapped = vm::map_phys_range(
            &mut PhysPageAllocator {},
            &mut VmTraitImpl {},
            debug_name,
            &PhysRange::with_pa_len(page_pa, PAGE_SIZE_4K),
            VaMapping::Addr(va),
            Entry::rw_user_data(),
            PageSize::Page4K,
            RootPageTableType::User,
        );
        if mapped.is_err() {
            port::pagealloc::free_physpage(page_pa).expect("couldn't free an unmapped page");
            return Err(PageAllocError::UnableToMap);
        }
    }
    Ok(())
}
