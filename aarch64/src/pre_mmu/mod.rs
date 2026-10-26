// Code in this module should only be called before the MMU is enabled.

mod util;
mod vminit;

/// Called once from l.S, at EL1 with the MMU off, after the BSS is cleared.
/// It sets up the early UART, then builds the initial page tables.
#[unsafe(no_mangle)]
pub extern "C" fn pre_mmu_init(dtb_pa: u64) {
    util::init_early_uart_rpi4();
    // Show we've got this far
    util::init_early_uart_putc(b'.');
    vminit::init_vm(dtb_pa);
}
