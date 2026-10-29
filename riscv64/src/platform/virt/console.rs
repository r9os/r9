extern crate alloc;

use alloc::boxed::Box;

use crate::uart16550::Uart16550;
use port::{console, fdt::DeviceTree};

pub fn init(dt: &DeviceTree) {
    // Without a UART, there's nowhere to report the failure, so carry on
    // without a console, as aarch64 does.
    let Some(ns16550a_reg) = dt
        .find_compatible("ns16550a")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
    else {
        return;
    };

    let mut uart = Uart16550::new(ns16550a_reg);
    uart.init(115_200);
    console::set_uart(Box::leak(Box::new(uart)));
}
