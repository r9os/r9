use crate::uartmini::{MiniUart, RX_FIFO};
use alloc::boxed::Box;
use port::console;
use port::fdt::DeviceTree;
use port::mcslock::{Lock, LockNode};
// The aarch64 console implementation is focussed on Raspberry Pi 4 for now.

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

/// The mini UART and its interrupt, once init has found both: the console's
/// input.  It's taken with ilock, since the interrupt handler takes it, and
/// held only to copy them out.
static INPUT: Lock<Option<(&'static MiniUart, usize)>> = Lock::new("miniuart input", None);

fn input() -> Option<(&'static MiniUart, usize)> {
    let node = LockNode::new();
    *INPUT.ilock(&node)
}

pub fn init(dt: &DeviceTree) {
    // Without a UART, there's nowhere to report the failure.
    let Ok(uart) = MiniUart::new_with_map_ranges(dt) else {
        return;
    };
    uart.init();
    let uart: &'static MiniUart = Box::leak(Box::new(uart));
    console::set_uart(uart);
    if let Ok(intid) = MiniUart::find_intid(dt) {
        let node = LockNode::new();
        *INPUT.ilock(&node) = Some((uart, intid));
    }
}

/// Take the mini UART's receive interrupt for the console's input, once the
/// GIC is set up.  Without a UART, or its interrupt, there's no input.
pub fn start() {
    let Some((uart, intid)) = input() else {
        return;
    };
    crate::gic::attach(intid, interrupt);
    uart.enable_receive_interrupt();
}

/// The mini UART's receive interrupt: drain its FIFO into the console's
/// input.  The interrupt is shared with the auxiliary SPIs, which r9 doesn't
/// use, so one that isn't the mini UART's has nothing to do.
fn interrupt() {
    let Some((uart, _)) = input() else {
        return;
    };
    if !uart.interrupt_pending() {
        return;
    }
    let mut buf = [0; RX_FIFO];
    let n = uart.drain(&mut buf);
    port::console::put_input(&buf[..n]);
}
