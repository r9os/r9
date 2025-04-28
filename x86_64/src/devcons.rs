// Racy to start.

use core::cell::SyncUnsafeCell;
use port::devcons::{Console, Uart};

struct Uart16550 {
    port: u16,
}

impl Uart for Uart16550 {
    fn putb(&self, b: u8) {
        crate::uart16550::putb(self.port, b);
    }
}

pub fn init() {
    Console::set_uart(|| {
        static CONS: SyncUnsafeCell<Uart16550> = SyncUnsafeCell::new(Uart16550 { port: 0x3f8 });
        unsafe { Ok(&mut *CONS.get()) }
    });
}
