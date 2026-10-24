//! Temporary kernel mappings of physical pages, as in Plan 9's kmap.
//!
//! There's no direct map of RAM, so the kernel reaches a page it doesn't
//! otherwise map (a user page, or a page table that isn't active) by mapping
//! it briefly in the kmap region.  Each CPU has its own KMAPSLOTS slots, so
//! CPUs never contend for them.  kmap is for thread context only: interrupt
//! handlers must not use it, since they could take a slot that the
//! interrupted code is about to claim.

use crate::param::{KMAPBASE, KMAPSLOTS, MAXMACH};
use crate::vm::{Entry, kernel_pte};
use core::marker::PhantomData;
use core::ptr::write_volatile;
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
        let va = self.va();
        // SAFETY: va is in this CPU's kmap region, whose level 3 table vminit
        // built, and this KMap owns the slot.
        unsafe { write_volatile(kernel_pte(va), Entry::empty()) };
        invalidate_kernel_va(va);
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

    let entry = Entry::rw_kernel_data().with_phys_addr(pa).with_page_or_table(true);
    // SAFETY: as in drop.  The slot was free, so the entry is invalid and no
    // TLB entry for it can exist: the TLB doesn't hold faulting translations.
    unsafe { write_volatile(kernel_pte(k.va()), entry) };
    #[cfg(not(test))]
    // SAFETY: barriers only.
    unsafe {
        // Make the entry visible to the table walker before the slot is used.
        core::arch::asm!("dsb ishst", "isb");
    }
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

/// Remove any TLB entry for the kernel address `va` on every CPU.
#[allow(unused_variables)]
fn invalidate_kernel_va(va: usize) {
    #[cfg(not(test))]
    // SAFETY: TLB maintenance and barriers only.
    unsafe {
        // Make the cleared entry visible to the table walker, then remove the
        // old translation.  Kernel entries are global, so invalidate for all
        // ASIDs (vaae1is).  The operand's bits [43:0] are VA[55:12]; the bits
        // above must be clear, as bits [47:44] are the TTL hint (Arm ARM,
        // TLBI VAAE1IS; Linux __TLBI_VADDR masks the same way).
        core::arch::asm!(
            "dsb ishst",
            "tlbi vaae1is, {page}",
            "dsb ish",
            "isb",
            page = in(reg) (va >> 12) & ((1 << 44) - 1),
        );
    }
}
