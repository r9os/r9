// Racy to start.

use crate::uartmini::{MiniUart, RX_FIFO};
use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};
use port::console::Console;
use port::fdt::DeviceTree;
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

static UART: SyncUnsafeCell<MaybeUninit<MiniUart>> = SyncUnsafeCell::new(MaybeUninit::uninit());

/// The mini UART's interrupt, or 0 until init has set UART and found it.
static INTID: AtomicUsize = AtomicUsize::new(0);

pub fn init(dt: &DeviceTree) {
    // Without a UART, there's nowhere to report the failure.
    let Ok(uart) = MiniUart::new_with_map_ranges(dt) else {
        return;
    };
    uart.init();

    // SAFETY: init runs once, before anything else uses UART.
    let uart = unsafe { (*UART.get()).write(uart) };
    Console::set_uart(uart);
    if let Ok(intid) = MiniUart::find_intid(dt) {
        INTID.store(intid, Ordering::Release);
    }
}

/// Take the mini UART's receive interrupt for the console's input, once the
/// GIC is set up.  Without a UART, or its interrupt, there's no input.
pub fn start() {
    let intid = INTID.load(Ordering::Acquire);
    if intid == 0 {
        return;
    }
    crate::gic::attach(intid, interrupt);
    // SAFETY: INTID is set only once init has written UART.
    unsafe { (*UART.get()).assume_init_ref() }.enable_receive_interrupt();
}

/// The mini UART's receive interrupt: drain its FIFO into the console's
/// input.  The interrupt is shared with the auxiliary SPIs, which r9 doesn't
/// use, so one that isn't the mini UART's has nothing to do.
fn interrupt() {
    // SAFETY: the interrupt is attached only once init has written UART.
    let uart = unsafe { (*UART.get()).assume_init_ref() };
    if !uart.interrupt_pending() {
        return;
    }
    let mut buf = [0; RX_FIFO];
    let (n, overrun) = uart.drain(&mut buf);
    port::console::put_input(&buf[..n], overrun);
}
