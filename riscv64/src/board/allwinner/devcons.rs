// Racy to start.

use core::mem::MaybeUninit;

use crate::uart16550::Uart16550;
use port::{devcons::Console, fdt::DeviceTree};

pub fn init(dt: &DeviceTree) {
    let uart0_reg = dt
        .find_compatible("uart0")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
        .unwrap();

    Console::new(|| {
        let mut uart = Uart16550::new(uart0_reg);
        uart.init(115_200);

        static mut UART: MaybeUninit<Uart16550> = MaybeUninit::uninit();

        unsafe {
            UART.write(uart);
            UART.assume_init_mut()
        }
    });
}
