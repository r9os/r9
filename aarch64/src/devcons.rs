// Racy to start.

use crate::uartmini::MiniUart;
use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;
use port::devcons::{Console, IprintOps, Uart};
use port::fdt::DeviceTree;
#[cfg(not(test))]
use port::println;
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

static UART: SyncUnsafeCell<MaybeUninit<MiniUart>> = SyncUnsafeCell::new(MaybeUninit::uninit());

static IPRINT_OPS: IprintOps = IprintOps { putb: iputb };

/// Direct polled write for iprint, bypassing the console lock.
/// `MiniUart::putb` needs only a shared reference, so this can safely
/// alias the reference held by the console.
fn iputb(b: u8) {
    // Safety: IPRINT_OPS is only registered once UART is initialised.
    let uart = unsafe { (*UART.get()).assume_init_ref() };
    uart.putb(b);
}

pub fn init(dt: &DeviceTree) {
    Console::set_uart(|| {
        let uart = MiniUart::new_with_map_ranges(dt);

        // Return a statically initialised MiniUart.  If that couldn't be done for some reason,
        // return None and hope that things work out regardless
        match uart {
            Ok(uart) => {
                uart.init();

                unsafe {
                    let cons = &mut *UART.get();
                    cons.write(uart);
                    port::devcons::set_iprint_ops(&IPRINT_OPS);
                    Ok(cons.assume_init_ref())
                }
            }
            Err(msg) => {
                println!("can't initialise uart: {msg:?}");
                Err("can't initialise uart")
            }
        }
    });
}
