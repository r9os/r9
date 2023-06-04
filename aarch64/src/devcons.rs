// Racy to start.

use crate::registers::MidrEl1;
use crate::uartmini::MiniUart;
use core::mem::MaybeUninit;
use port::devcons::LockingConsole;
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
    LockingConsole::new(|| {
        const KZERO: u64 = 0xffff800000000000;
        let base = KZERO + MidrEl1::read().partnum_enum().map(|p| p.mmio()).unwrap_or(0);
        let gpio_addr = base + 0x200000;
        let aux_addr = base + 0x215000;
        let miniuart_addr = base + 0x215040;

        let uart = MiniUart::from_addresses(gpio_addr, aux_addr, miniuart_addr);
        //let uart = MiniUart::new(dt);
        // uart.init();

        static mut UART: MaybeUninit<MiniUart> = MaybeUninit::uninit();
        unsafe {
            UART.write(uart);
            UART.assume_init_mut()
        }
    });
}
