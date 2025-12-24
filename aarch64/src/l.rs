// Functions to be called in early bootup phase, typically before the MMU has been enabled.
// This shouldn't normally be used for anything other than debugging at very early init.

use core::ops::{BitAndAssign, BitOrAssign};
use core::ptr::{read_volatile, write_volatile};

const MMIO_BASE: u32 = 0xfe000000;
const AUX: u32 = MMIO_BASE + 0x00215000;
const AUX_ENABLES: u32 = AUX + 0x04;
const AUX_MU: u32 = AUX + 0x40;
const AUX_MU_IO: u32 = AUX_MU + 0x00; // AUX IO data register
const AUX_MU_IER: u32 = AUX_MU + 0x04;
const AUX_MU_IIR: u32 = AUX_MU + 0x08;
const AUX_MU_LCR: u32 = AUX_MU + 0x0c;
const AUX_MU_MCR: u32 = AUX_MU + 0x10;
const AUX_MU_CNTL: u32 = AUX_MU + 0x20;
const AUX_MU_BAUD: u32 = AUX_MU + 0x28;

// AUX_MU_MCR			= AUX_MU + 0x10
const AUX_MU_LSR: u32 = AUX_MU + 0x14;

const GPIO: u32 = MMIO_BASE + 0x00200000; // Offset from MMIO base
const GPFSEL1: u32 = GPIO + 0x04;
// GPPUD				= GPIO + 0x94
// GPPUDCLK0			= GPIO + 0x98
const GPIO_PUP_PDN_CNTRL_REG0: u32 = GPIO + 0xe4;

// Set up a very early uart - the miniuart.  The full driver is in
// uartmini.rs.  This code is just enough to help debug the early stage.
#[unsafe(no_mangle)]
pub extern "C" fn init_early_uart_rpi4() {
    // Calculate the baudrate to be inserted into AUX_MU_BAUD
    const UART_CLOCK: u32 = 500000000;
    const UART_BAUDRATE: u32 = 115200;
    const UART_BAUDRATE_REG: u32 = (UART_CLOCK / (UART_BAUDRATE * 8)) - 1;

    unsafe {
        write_or_volatile(AUX_ENABLES as *mut u32, 1);
        write_volatile(AUX_MU_CNTL as *mut u32, 0);
        write_volatile(AUX_MU_LCR as *mut u32, 3);
        write_volatile(AUX_MU_MCR as *mut u32, 0);
        write_volatile(AUX_MU_IER as *mut u32, 0);
        write_volatile(AUX_MU_IIR as *mut u32, 0xc6);
        write_volatile(AUX_MU_BAUD as *mut u32, UART_BAUDRATE_REG);
    }

    unsafe {
        // Set up GPIO pin 14 pull up/down state
        // Mask all but bits 28:29
        write_and_volatile(GPIO_PUP_PDN_CNTRL_REG0 as *mut u32, 0xcfffffff);

        // Set GPIO pins 14 to be used for ALT5 - UART1 (miniuart)
        let mut gpfsel1_val = read_volatile(GPFSEL1 as *const u32);
        // Mask all but bits 12:14 (pin 14)
        gpfsel1_val &= 0xffff8fff;
        // Pin 14, ALT5
        gpfsel1_val |= 0x00002000;
        write_volatile(GPFSEL1 as *mut u32, gpfsel1_val);

        // Set up GPIO pin 15 pull up/down state
        // Mask all but bits 30:31
        write_and_volatile(GPIO_PUP_PDN_CNTRL_REG0 as *mut u32, 0x3fffffff);

        // Set GPIO pins 15 to be used for ALT5 - UART1 (miniuart)
        let mut gpfsel1_val = read_volatile(GPFSEL1 as *const u32);
        // Mask all but bits 15:17 (pin 15)
        gpfsel1_val &= 0xfffc7fff;
        // Pin 15, ALT5
        gpfsel1_val |= 0x00010000;
        write_volatile(GPFSEL1 as *mut u32, gpfsel1_val);
    }

    unsafe {
        write_volatile(AUX_MU_CNTL as *mut u32, 3);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn init_early_uart_putc(b: u8) {
    unsafe {
        while read_volatile(AUX_MU_LSR as *const u32) & (1 << 5) == 0 {
            core::hint::spin_loop();
        }
        write_volatile(AUX_MU_IO as *mut u32, b as u32);
    }
}

unsafe fn write_or_volatile<T: BitOrAssign>(dst: *mut T, src: T) {
    unsafe {
        let mut new_val = read_volatile(dst);
        new_val |= src;
        write_volatile(dst, new_val);
    }
}

unsafe fn write_and_volatile<T: BitAndAssign>(dst: *mut T, src: T) {
    unsafe {
        let mut new_val = read_volatile(dst);
        new_val &= src;
        write_volatile(dst, new_val);
    }
}
