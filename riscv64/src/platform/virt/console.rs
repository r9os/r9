// Racy to start.

use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;

use crate::uart16550::Uart16550;
use port::{console::Console, fdt::DeviceTree};

pub fn init(dt: &DeviceTree) {
    let ns16550a_reg = dt
        .find_compatible("ns16550a")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
        .unwrap();

    let mut uart = Uart16550::new(ns16550a_reg);
    uart.init(115_200);

    static CONS: SyncUnsafeCell<MaybeUninit<Uart16550>> =
        SyncUnsafeCell::new(MaybeUninit::uninit());
    // SAFETY: init runs once, before anything else uses CONS.
    let uart = unsafe { (*CONS.get()).write(uart) };
    Console::set_uart(uart);
}
