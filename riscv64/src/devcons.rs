// Racy to start.

use crate::uart16550::Uart16550;
use port::devcons::Console;

pub fn init() {
    Console::new(|| {
        static mut UART: Uart16550 = Uart16550::new(0x1000_0000);
        unsafe {
            UART.init(115_200);
            &mut UART
        }
    });
}
