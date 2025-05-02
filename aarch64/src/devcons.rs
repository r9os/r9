// Racy to start.

use crate::param::KZERO;
use crate::uartmini::MiniUart;
use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;
use port::devcons::Console;
use port::fdt::DeviceTree;
#[cfg(not(test))]
use port::println;
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

pub fn init(dt: &DeviceTree, is_early_init: bool) {
    Console::set_uart(|| {
        let uart = if is_early_init {
            MiniUart::new_assuming_mapped_mmio(dt, KZERO)
        } else {
            MiniUart::new_with_map_ranges(dt)
        };

        // Return a statically initialised MiniUart.  If that couldn't be done for some reason,
        // return None and hope that things work out regardless
        match uart {
            Ok(uart) => {
                uart.init();

                static UART: SyncUnsafeCell<MaybeUninit<MiniUart>> =
                    SyncUnsafeCell::new(MaybeUninit::uninit());
                unsafe {
                    let cons = &mut *UART.get();
                    cons.write(uart);
                    Ok(cons.assume_init_mut())
                }
            }
            Err(msg) => {
                println!("can't initialise uart: {msg:?}");
                Err("can't initialise uart")
            }
        }
    });
}
