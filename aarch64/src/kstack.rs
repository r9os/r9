//! Kernel stacks, each with an unmapped guard below it, in the kernel stack
//! region (see KSTACKBASE).  An overflow runs into the guard and faults,
//! rather than corrupting whatever is mapped below the stack.

use crate::param::{KSTACKBASE, KSTACKSIZE, KSTACKSLOTS};
use crate::vm::{map_kernel_page, unmap_kernel_page};
use core::sync::atomic::{AtomicBool, Ordering};
use port::mem::PAGE_SIZE_4K;
use port::pagealloc::{PageAllocError, allocate_physpage, free_physpage};

/// Which slots are in use.  Claiming a slot (Acquire) is ordered after the
/// previous owner's unmap and TLB invalidate (Release).
static INUSE: [AtomicBool; KSTACKSLOTS] = [const { AtomicBool::new(false) }; KSTACKSLOTS];

const PAGES: usize = KSTACKSIZE / PAGE_SIZE_4K;

/// A kernel stack.  It isn't Clone, and it isn't freed when dropped, since a
/// suspended context may still be on it: see free.
pub struct KStack {
    slot: usize,
}

impl KStack {
    /// The lowest address of the stack.  The KSTACKSIZE below it is the guard.
    fn base(&self) -> usize {
        slot_base(self.slot)
    }

    /// The top of the stack, which is 16-byte aligned.
    pub fn top(&self) -> usize {
        self.base() + KSTACKSIZE
    }
}

/// The lowest address of slot `slot`'s stack.
fn slot_base(slot: usize) -> usize {
    KSTACKBASE + slot * 2 * KSTACKSIZE + KSTACKSIZE
}

/// Claim a free slot, or return None if all are in use.
fn claim_slot() -> Option<usize> {
    INUSE
        .iter()
        .position(|s| s.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok())
}

/// Allocate a zeroed kernel stack.
pub fn alloc() -> Result<KStack, PageAllocError> {
    let stack = KStack { slot: claim_slot().ok_or(PageAllocError::OutOfSpace)? };
    for i in 0..PAGES {
        let pa = match allocate_physpage() {
            Ok(pa) => pa,
            Err(err) => {
                // SAFETY: the first i pages were just mapped, and nothing uses
                // them yet.
                unsafe { unmap_and_free(&stack, i) };
                INUSE[stack.slot].store(false, Ordering::Release);
                return Err(err);
            }
        };
        // SAFETY: vminit built the region's level 3 table, and the slot was
        // free, so the entry is invalid.
        unsafe { map_kernel_page(stack.base() + i * PAGE_SIZE_4K, pa) };
    }
    // Zero the stack through its own, now mapped, address: zeroing each page
    // through kmap would cost a TLB invalidate per page.
    // SAFETY: the whole stack was just mapped read-write, and nothing else
    // refers to it.
    unsafe { core::ptr::write_bytes(stack.base() as *mut u8, 0, KSTACKSIZE) };
    Ok(stack)
}

/// Free `stack`.
///
/// # Safety
///
/// No context may be running on the stack, or be switched to on it later.
pub unsafe fn free(stack: KStack) {
    // SAFETY: the caller guarantees nothing uses the stack.
    unsafe { unmap_and_free(&stack, PAGES) };
    INUSE[stack.slot].store(false, Ordering::Release);
}

/// Unmap the first `n` pages of `stack`, and free them.
///
/// # Safety
///
/// Those pages must be mapped, and nothing may still use them.
unsafe fn unmap_and_free(stack: &KStack, n: usize) {
    for i in 0..n {
        // SAFETY: the caller guarantees the page is mapped and unused.
        let pa = unsafe { unmap_kernel_page(stack.base() + i * PAGE_SIZE_4K) };
        free_physpage(pa).expect("kstack: couldn't free a stack page");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_fit_the_region_and_are_guarded() {
        for slot in 0..KSTACKSLOTS {
            let base = slot_base(slot);
            let slot_start = KSTACKBASE + slot * 2 * KSTACKSIZE;
            // The guard is the lower half of the slot, the stack the upper.
            assert_eq!(base - slot_start, KSTACKSIZE);
            assert!(slot_start.is_multiple_of(2 * KSTACKSIZE));
            // The top is 16-byte aligned and within the region.
            let top = KStack { slot }.top();
            assert!(top.is_multiple_of(16));
            assert!(top <= KSTACKBASE + (2 << 20));
            // A valid sp in the stack has bit 14 set, and one in the guard
            // has it clear, which an overflow check at exception entry can
            // rely on.
            assert_ne!(base & KSTACKSIZE, 0);
            assert_eq!((base - 8) & KSTACKSIZE, 0);
        }
    }

    #[test]
    fn claims_run_out_after_every_slot() {
        let slots: Vec<usize> = (0..KSTACKSLOTS).map(|_| claim_slot().unwrap()).collect();
        assert_eq!(slots, (0..KSTACKSLOTS).collect::<Vec<_>>());
        assert_eq!(claim_slot(), None);
        for slot in slots {
            INUSE[slot].store(false, Ordering::Release);
        }
    }
}
