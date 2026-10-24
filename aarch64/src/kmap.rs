//! Temporary kernel mappings of physical pages, as in Plan 9's kmap.
//!
//! There's no direct map of RAM, so the kernel reaches a page it doesn't
//! otherwise map (a user page, or a page table that isn't active) by mapping
//! it briefly in the kmap region.  Each CPU has its own KMAPSLOTS slots, so
//! CPUs never contend for them.  kmap is for thread context only: interrupt
//! handlers must not use it, since they could take a slot that the
//! interrupted code is about to claim.

use crate::param::{KMAPBASE, KMAPSLOTS, MAXMACH};
use crate::vm::{map_kernel_page, unmap_kernel_page};
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};
use port::mem::{PAGE_SIZE_4K, PhysAddr};

/// Which of each CPU's slots are in use.  Claiming a slot (Acquire) is
/// ordered after the previous owner's unmap and TLB invalidate (Release), so
/// a new owner never sees the old mapping.
static INUSE: [[AtomicBool; KMAPSLOTS]; MAXMACH] =
    [const { [const { AtomicBool::new(false) }; KMAPSLOTS] }; MAXMACH];

/// A page mapped by kmap.  Dropping it unmaps the page.  It belongs to the
/// CPU that mapped it, so it can't be sent to another.
pub struct KMap {
    machno: usize,
    slot: usize,
    _not_send: PhantomData<*const ()>,
}

impl KMap {
    /// The kernel address of the mapped page.
    pub fn va(&self) -> usize {
        slot_va(self.machno, self.slot)
    }
}

impl Drop for KMap {
    fn drop(&mut self) {
        // SAFETY: the address is in this CPU's kmap region, whose level 3
        // table vminit built, and this KMap owns the slot and its mapping.
        unsafe { unmap_kernel_page(self.va()) };
        INUSE[self.machno][self.slot].store(false, Ordering::Release);
    }
}

/// Map the physical page at `pa` into one of this CPU's kmap slots.  Panics
/// if all of them are in use, which means more than KMAPSLOTS nested kmaps.
///
/// `pa` must be RAM.  The page is mapped as normal cacheable memory, so
/// kmapping a device register would create a cacheable alias of it, which
/// the architecture doesn't keep coherent with its Device mapping.  Map
/// devices through deviceutil instead.
pub fn kmap(pa: PhysAddr) -> KMap {
    let machno = machno();
    let slot = INUSE[machno]
        .iter()
        .position(|s| s.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok())
        .expect("kmap: all slots in use");
    let k = KMap { machno, slot, _not_send: PhantomData };

    // SAFETY: the address is in this CPU's kmap region, whose level 3 table
    // vminit built, and the slot was free, so its entry is invalid.
    unsafe { map_kernel_page(k.va(), pa) };
    k
}

fn slot_va(machno: usize, slot: usize) -> usize {
    KMAPBASE + (machno * KMAPSLOTS + slot) * PAGE_SIZE_4K
}

/// Return this CPU's number, Aff0 of MPIDR_EL1 (one cluster of up to four
/// cores on the Raspberry Pi 4).
fn machno() -> usize {
    #[cfg(not(test))]
    let machno = {
        use aarch64_cpu::registers::{MPIDR_EL1, Readable};
        MPIDR_EL1.read(MPIDR_EL1::Aff0) as usize
    };
    #[cfg(test)]
    let machno = 0;
    assert!(machno < MAXMACH, "machno {machno} >= MAXMACH");
    machno
}
