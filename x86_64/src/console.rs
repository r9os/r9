use port::console::{self, Uart};

struct Uart16550 {
    port: u16,
}

impl Uart for Uart16550 {
    fn putb(&self, b: u8) {
        crate::uart16550::putb(self.port, b);
    }
}

static CONS: Uart16550 = Uart16550 { port: 0x3f8 };

pub fn init() {
    console::set_uart(&CONS);
}
