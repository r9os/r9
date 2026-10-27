//! The generic timer's counter.  l.S puts the timer's controls in a known
//! state.

#[cfg(not(test))]
use aarch64_cpu::registers::{CNTFRQ_EL0, CNTVCT_EL0, Readable};

/// port::clock's functions.  Reads of CNTVCT_EL0 are in program order
/// relative to each other, but can be taken ahead of earlier instructions
/// (Arm ARM DDI 0487 M.d, D12.2.2), so an isb keeps a time read after some
/// work from being read before it, as Linux's __arch_counter_get_cntvct
/// does.  Firmware sets CNTFRQ_EL0 (D24.10.1).
#[cfg(not(test))]
static CLOCK: port::clock::Ops = port::clock::Ops {
    fastticks: || {
        aarch64_cpu::asm::barrier::isb(aarch64_cpu::asm::barrier::SY);
        CNTVCT_EL0.get()
    },
    fasthz: || CNTFRQ_EL0.get(),
};

/// Register the counter with port::clock.
pub fn init() {
    #[cfg(not(test))]
    port::clock::init(&CLOCK);
}
