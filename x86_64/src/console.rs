// Racy to start.

use core::cell::SyncUnsafeCell;
use port::console::{Console, Uart};

struct Uart16550 {
    port: u16,
}

impl Uart for Uart16550 {
    fn putb(&self, b: u8) {
        crate::uart16550::putb(self.port, b);
    }
}

pub fn init() {
    static CONS: SyncUnsafeCell<Uart16550> = SyncUnsafeCell::new(Uart16550 { port: 0x3f8 });
    // SAFETY: init runs once, before anything else uses CONS.
    Console::set_uart(unsafe { &mut *CONS.get() });
}
