// Racy to start.

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
    Console::new(|| {
        static mut UART: Uart16550 = Uart16550 { port: 0x3f8 };
        unsafe { &mut UART }
    });
}
