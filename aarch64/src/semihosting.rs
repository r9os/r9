//! Exiting QEMU through Arm semihosting, for the smoke test.  It's only built
//! with the `semihosting` feature: on hardware without a debugger, the
//! semihosting call is an undefined instruction.

use core::sync::atomic::{AtomicBool, Ordering};

/// SYS_EXIT, and the reason that makes QEMU exit with the given code (Arm's
/// semihosting specification, SYS_EXIT; QEMU's semihosting/arm-compat-semi.c).
const SYS_EXIT: u64 = 0x18;
const ADP_STOPPED_APPLICATION_EXIT: u64 = 0x20026;

static EXITING: AtomicBool = AtomicBool::new(false);

/// Make QEMU exit with `code`.  If QEMU wasn't started with semihosting, the
/// call faults, and the panic handler calls this again: the second call parks.
pub fn exit(code: u64) -> ! {
    if !EXITING.swap(true, Ordering::Relaxed) {
        // On AArch64, SYS_EXIT's parameter is a pointer to {reason, code}.
        let block = [ADP_STOPPED_APPLICATION_EXIT, code];
        // SAFETY: HLT #0xf000 is the A64 semihosting call.  QEMU reads the
        // block, which outlives the call, and exits.
        unsafe {
            core::arch::asm!(
                "hlt #0xf000",
                inout("x0") SYS_EXIT => _,
                in("x1") block.as_ptr(),
                options(nostack),
            )
        };
    }
    crate::halt()
}
