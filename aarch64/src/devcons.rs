// Racy to start.

use crate::uartmini::MiniUart;
use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;
use port::devcons::Console;
use port::fdt::DeviceTree;
// The aarch64 devcons implementation is focussed on Raspberry Pi 4 for now.

// Useful links
// - Raspberry Pi Processors
//     https://www.raspberrypi.com/documentation/computers/processors.html
// - Raspberry Pi Hardware
//     https://www.raspberrypi.com/documentation/computers/raspberry-pi.html
// - Raspi4 BCM2711
//     Datasheet https://datasheets.raspberrypi.com/bcm2711/bcm2711-peripherals.pdf
// - Mailbox
//     https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface

// Raspberry Pi 4 has 4 UARTs:
// - UART0 PL011
// - UART1 miniUART
// - UART2 PL011
// - UART3 PL011

pub fn init(dt: &DeviceTree) {
    // Without a UART, there's nowhere to report the failure.
    let Ok(uart) = MiniUart::new_with_map_ranges(dt) else {
        return;
    };
    uart.init();

    static UART: SyncUnsafeCell<MaybeUninit<MiniUart>> = SyncUnsafeCell::new(MaybeUninit::uninit());
    // SAFETY: init runs once, before anything else uses UART.
    let uart = unsafe { (*UART.get()).write(uart) };
    Console::set_uart(uart);
}
