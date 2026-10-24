//! Per-CPU state, as Plan 9's Mach.  Each CPU's TPIDR_EL1 holds the address of
//! its Mach.

use crate::param::MAXMACH;
use crate::proc::Arm64;
use crate::swtch::Label;
use core::cell::SyncUnsafeCell;
use port::dat::Proc;

pub struct Mach {
    /// The current process, or null.
    pub up: *mut Proc<Arm64>,
    /// The scheduler's context, which a process switches back to when it
    /// exits.
    pub sched: Label,
}

// SAFETY: each Mach is only used by its own CPU.
unsafe impl Sync for Mach {}

static MACH: [SyncUnsafeCell<Mach>; MAXMACH] =
    [const { SyncUnsafeCell::new(Mach { up: core::ptr::null_mut(), sched: Label::new() }) };
        MAXMACH];

/// Point this CPU's TPIDR_EL1 at its Mach.
pub fn init() {
    #[cfg(not(test))]
    {
        use aarch64_cpu::registers::{TPIDR_EL1, Writeable};
        TPIDR_EL1.set(MACH[machno()].get().addr() as u64);
    }
}

/// This CPU's Mach.
pub fn mach() -> *mut Mach {
    #[cfg(not(test))]
    {
        use aarch64_cpu::registers::{Readable, TPIDR_EL1};
        TPIDR_EL1.get() as *mut Mach
    }
    #[cfg(test)]
    MACH[0].get()
}

/// Return this CPU's number, Aff0 of MPIDR_EL1 (one cluster of up to four
/// cores on the Raspberry Pi 4).
pub fn machno() -> usize {
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
