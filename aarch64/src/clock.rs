//! The generic timer.

#[cfg(not(test))]
use aarch64_cpu::registers::{CNTKCTL_EL1, CNTV_CTL_EL0, Writeable};

/// Put the EL1 timer registers in a known state.  Both reset to UNKNOWN
/// values (Arm ARM DDI 0487 M.d, D24.10.15 and D24.10.28).  CNTKCTL_EL1 = 0
/// traps EL0's reads of the counters and timers, and CNTV_CTL_EL0 = 0
/// disables the virtual timer, which could otherwise be armed with a stale
/// deadline.
pub fn init() {
    #[cfg(not(test))]
    {
        CNTKCTL_EL1.set(0);
        CNTV_CTL_EL0.set(0);
    }
}
