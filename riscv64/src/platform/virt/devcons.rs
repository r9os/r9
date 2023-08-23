// Racy to start.

use core::mem::MaybeUninit;

use crate::sbi::Sbi;
use crate::uart16550::Uart16550;
use port::{devcons::Console, fdt::DeviceTree};

pub fn init(dt: &DeviceTree) {
    let ns16550a_reg = dt
        .find_compatible("ns16550a")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
        .unwrap();

    Console::new(|| {
        let addr = ns16550a_reg.addr as usize;
        let mut uart = Uart16550::new(addr);

        uart.init(115_200);

        static mut UART: MaybeUninit<Uart16550> = MaybeUninit::uninit();

        unsafe {
            UART.write(uart);
            UART.assume_init_mut()
        }
    });
}

pub fn init_sbi() {
    Console::new(|| {
        let uart = Sbi::new();

        static mut UART: MaybeUninit<Sbi> = MaybeUninit::uninit();

        unsafe {
            UART.write(uart);
            UART.assume_init_mut()
        }
    });
}
