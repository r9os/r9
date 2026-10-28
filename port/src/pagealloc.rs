//! The kernel's physical page allocator, shared by every architecture.
//!
//! The page allocator is constructed and finalised in a number of phases:
//! 1. `PAGE_ALLOC` starts as a fixed size allocator with every page marked as
//!    in use, so nothing can be allocated before the memory map is known.
//! 2. `init_page_allocator` marks the DTB's first memory region as free,
//!    except for the ranges already in use (the kernel image, the DTB, the
//!    early page tables).

use crate::bitmapalloc::BitmapPageAlloc;
use crate::fdt::DeviceTree;
use crate::irq;
use crate::mcslock::{Lock, LockGuard, LockNode};
use crate::mem::{PAGE_SIZE_4K, PhysAddr, PhysRange};

#[cfg(not(test))]
use crate::println;

/// General page allocation errors.  Not specific to any particular implementation, and also includes higher-level errors.
#[derive(Debug, PartialEq)]
pub enum PageAllocError {
    OutOfBounds,
    MisalignedAddr,
    OutOfSpace,
    NotAllocated,
    UnableToMap,
}

/// Set up bitmap page allocator assuming everything is allocated.
static PAGE_ALLOC: Lock<BitmapPageAlloc<32, PAGE_SIZE_4K>> = Lock::new(
    "page_alloc",
    const { BitmapPageAlloc::<32, PAGE_SIZE_4K>::new_all_allocated(PAGE_SIZE_4K) },
);

/// Take PAGE_ALLOC.  Its lock doesn't mask IRQs, so an interrupt handler that
/// took it while the code it interrupted held it would deadlock: handlers
/// mustn't allocate pages.
fn page_alloc(node: &LockNode) -> LockGuard<'_, BitmapPageAlloc<32, PAGE_SIZE_4K>> {
    assert!(!irq::in_interrupt(), "page allocation in interrupt context");
    PAGE_ALLOC.lock(node)
}

/// Free the DTB's first memory region, except for `used_ranges`, which must be
/// sorted.
pub fn init_page_allocator(
    dt: &DeviceTree,
    used_ranges: impl Iterator<Item = PhysRange>,
) -> Result<(), PageAllocError> {
    let Some(available_mem) = dt
        .find_device_type("memory")
        .flat_map(|memory| dt.property_translated_reg_iter(memory).flat_map(|r| r.regblock()))
        .map(|memory| PhysRange::from(&memory))
        .next()
    else {
        panic!("error:pagealloc:free_unused_ranges: can't get memory from dtb\n");
    };

    let node = LockNode::new();
    let mut lock = page_alloc(&node);
    let page_alloc = &mut *lock;

    page_alloc.free_unused_ranges(available_mem, used_ranges)
}

/// Try to allocate a physical page.  Note that this is NOT mapped.
pub fn allocate_physpage() -> Result<PhysAddr, PageAllocError> {
    let node = LockNode::new();
    let mut lock = page_alloc(&node);
    let page_alloc = &mut *lock;

    match page_alloc.allocate() {
        Ok(page_pa) => {
            //println!("pagealloc:allocate_physpage pa:{:?}", page_pa);
            Ok(page_pa)
        }
        Err(err) => {
            println!("error:pagealloc:allocate_physpage:failed to allocate: {:?}", err);
            Err(err)
        }
    }
}

/// Free a physical page from allocate_physpage.  Nothing may still map it.
pub fn free_physpage(pa: PhysAddr) -> Result<(), PageAllocError> {
    let node = LockNode::new();
    let mut lock = page_alloc(&node);
    let page_alloc = &mut *lock;

    page_alloc.deallocate(pa).inspect_err(|err| {
        println!("error:pagealloc:free_physpage:failed to free {pa:?}: {err:?}");
    })
}

/// Return a tuple of (bytes used, total bytes available) based on the page allocator.
pub fn usage_bytes() -> (usize, usize) {
    let node = LockNode::new();
    let mut lock = page_alloc(&node);
    let page_alloc = &mut *lock;
    page_alloc.usage_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "page allocation in interrupt context")]
    fn allocating_in_interrupt_context_panics() {
        irq::init(&irq::tests::FAKE);
        irq::enter_interrupt();
        let _ = allocate_physpage();
    }
}
