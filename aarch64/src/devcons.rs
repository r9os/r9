// Racy to start.

use crate::param::KZERO;
use crate::uartmini::MiniUart;
use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;
use port::devcons::Console;
use port::fdt::DeviceTree;

// The aarch64 devcons implementation is focussed on Raspberry Pi 3, 4 for now.

// Useful links
// - Raspberry Pi Processors
//     https://www.raspberrypi.com/documentation/computers/processors.html
// - Raspberry Pi Hardware
//     https://www.raspberrypi.com/documentation/computers/raspberry-pi.html
// - Raspi3 BCM2837
//     Datasheet (BCM2835) https://datasheets.raspberrypi.com/bcm2835/bcm2835-peripherals.pdf
// - Raspi4 BCM2711
//     Datasheet https://datasheets.raspberrypi.com/bcm2711/bcm2711-peripherals.pdf
// - Mailbox
//     https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface

// Raspberry Pi 3 has 2 UARTs, Raspbery Pi 4 has 4:
// - UART0 PL011
// - UART1 miniUART
// - UART2 PL011 (rpi4)
// - UART3 PL011 (rpi4)

// TODO
// - Detect board type and set MMIO base address accordingly
//     https://wiki.osdev.org/Detecting_Raspberry_Pi_Board
// - Break out mailbox, gpio code

pub fn init(dt: &DeviceTree) {
    Console::new(|| {
        let uart = MiniUart::new(dt, KZERO);
        uart.init();

        static UART: SyncUnsafeCell<MaybeUninit<MiniUart>> =
            SyncUnsafeCell::new(MaybeUninit::uninit());
        unsafe {
            let cons = &mut *UART.get();
            cons.write(uart);
            cons.assume_init_mut()
        }
    });
}
