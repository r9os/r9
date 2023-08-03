// Racy to start.

use crate::registers::rpi_mmio;
use crate::uartmini::MiniUart;
use core::mem::MaybeUninit;
use port::devcons::Console;
use port::fdt::DeviceTree;
use port::mem::VirtRange;

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

pub fn init(_dt: &DeviceTree) {
    Console::new(|| {
        let mmio = rpi_mmio().expect("mmio base detect failed").to_virt();

        let gpio_range = VirtRange(mmio + 0x20_0000..mmio + 0x20_00b4);
        let aux_range = VirtRange(mmio + 0x21_5000..mmio + 0x21_5008);
        let miniuart_range = VirtRange(mmio + 0x21_5040..mmio + 0x21_5080);

        let uart = MiniUart { gpio_range, aux_range, miniuart_range };
        //let uart = MiniUart::new(dt);
        // uart.init();

        static mut UART: MaybeUninit<MiniUart> = MaybeUninit::uninit();
        unsafe {
            UART.write(uart);
            UART.assume_init_mut()
        }
    });
}
