//! User memory, as Plan 9's port/segment.c.  There are no segments yet, only
//! pages mapped in the current address space.

use crate::dat::{MMU, Perm, PutError};
use crate::mem::{PAGE_SIZE_4K, VirtRange};
use crate::pagealloc::{allocate_physpage, free_physpage};

/// Allocate, zero and map a data page for each 4KiB page in `range`, in the
/// current user address space.  Each page is zeroed before it's mapped, so a
/// process never sees another's data.  If it fails partway, the pages it
/// mapped are unmapped and freed again.
pub fn map_zeroed<A: MMU>(range: VirtRange) -> Result<(), PutError> {
    assert!(
        range.start.is_multiple_of(PAGE_SIZE_4K) && range.end.is_multiple_of(PAGE_SIZE_4K),
        "map_zeroed: range {range} isn't page aligned"
    );
    for va in (range.start..range.end).step_by(PAGE_SIZE_4K) {
        let mapped = allocate_physpage().map_err(|_| PutError::NoMemory).and_then(|pa| {
            // SAFETY: f gets the whole of the newly allocated page, which
            // nothing else refers to.
            A::kmap(pa, |p| unsafe { core::ptr::write_bytes(p, 0, PAGE_SIZE_4K) });
            A::put(va, pa, Perm::Data).inspect_err(|_| {
                free_physpage(pa).expect("map_zeroed: couldn't free an unmapped page")
            })
        });
        if let Err(err) = mapped {
            unmap_and_free::<A>(VirtRange::new(range.start, va));
            return Err(err);
        }
    }
    Ok(())
}

/// Unmap and free each 4KiB page mapped in `range`, in the current user
/// address space.  Every page mapped there must belong to the process.
pub fn unmap_and_free<A: MMU>(range: VirtRange) {
    for va in (range.start..range.end).step_by(PAGE_SIZE_4K) {
        if let Some(pa) = A::unput(va) {
            free_physpage(pa).expect("unmap_and_free: couldn't free a page");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, mapped};

    #[test]
    fn map_zeroed_without_memory_leaves_nothing_mapped() {
        // The host page allocator has nothing free.
        let range = VirtRange::new(0x10000, 0x13000);
        assert_eq!(map_zeroed::<Fake>(range), Err(PutError::NoMemory));
        assert!(mapped().is_empty());
    }

    #[test]
    fn unmap_and_free_skips_unmapped_pages() {
        unmap_and_free::<Fake>(VirtRange::new(0x10000, 0x13000));
        assert!(mapped().is_empty());
    }
}
