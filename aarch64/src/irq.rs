//! DAIF-based implementation of the portable interrupt masking hooks.

use port::irq::IrqOps;

static IRQ_OPS: IrqOps = IrqOps { mask: mask_irqs, restore: restore_irqs };

/// Register DAIF masking with `port::irq`.  Must be called before
/// interrupts are enabled.
pub fn init() {
    port::irq::set_ops(&IRQ_OPS);
}

/// Mask IRQs on this core, returning the previous DAIF state.
fn mask_irqs() -> u64 {
    let daif: u64;
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            "msr daifset, #2",
            daif = out(reg) daif,
            options(nostack, preserves_flags)
        );
    }
    daif
}

/// Restore a DAIF state previously returned by `mask_irqs`.
fn restore_irqs(daif: u64) {
    unsafe {
        core::arch::asm!(
            "msr daif, {daif}",
            daif = in(reg) daif,
            options(nostack, preserves_flags)
        );
    }
}
