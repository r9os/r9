// Racy to start.

use core::mem;
use core::mem::MaybeUninit;
use core::ptr;
use core::ptr::{read_volatile, write_volatile};
use port::devcons::{Console, Uart};
use port::fdt::{DeviceTree, RegBlock};

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

// GPIO registers
const GPPUD: u64 = 0x94;
const GPPUDCLK0: u64 = 0x98;

// UART 0 (PL011) registers
const UART0_DR: u64 = 0x00; // Data register
const UART0_FR: u64 = 0x18; // Flag register
const UART0_IBRD: u64 = 0x24; // Integer baud rate divisor
const UART0_FBRD: u64 = 0x28; // Fractional baud rate divisor
const UART0_LCRH: u64 = 0x2c; // Line control register
const UART0_CR: u64 = 0x30; // Control register
const UART0_IMSC: u64 = 0x38; // Interrupt mask set clear register
const UART0_ICR: u64 = 0x44; // Interrupt clear register

const MBOX_READ: u64 = 0x00;
const MBOX_STATUS: u64 = 0x18;
const MBOX_WRITE: u64 = 0x20;

const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;

// Delay for count cycles
fn delay(count: u32) {
    for _ in 0..count {
        core::hint::spin_loop();
    }
}

/// Write val into the reg RegBlock at offset from reg.addr.
/// Panics if offset is outside any range specified by reg.len.
fn write_reg(reg: RegBlock, offset: u64, val: u32) {
    let dst = reg.addr + offset;
    assert!(reg.len.map_or(true, |len| offset < len));
    unsafe { write_volatile(dst as *mut u32, val) }
}

/// Read from the reg RegBlock at offset from reg.addr.
/// Panics if offset is outside any range specified by reg.len.
fn read_reg(reg: RegBlock, offset: u64) -> u32 {
    let src = reg.addr + offset;
    assert!(reg.len.map_or(true, |len| offset < len));
    unsafe { read_volatile(src as *const u32) }
}

#[repr(u32)]
enum TagId {
    GetClockRate = 0x38002,
}

#[repr(u8)]
enum ChannelId {
    ArmToVc = 8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateRequest {
    size: u32, // size in bytes
    code: u32, // request code (0)

    // Tag
    tag_id0: u32,
    tag_buffer_size0: u32,
    tag_code0: u32,
    clock_id: u32,
    rate_hz: u32,
    skip_setting_turbo: u32,
    // No tag padding
    end_tag: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateResponse {
    size: u32, // size in bytes
    code: u32, // response code

    // Tag
    tag_id0: u32,
    tag_buffer_size0: u32,
    tag_code0: u32,
    clock_id: u32,
    rate_hz: u32,
    // No tag padding
    end_tag: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
union SetClockRate {
    request: SetClockRateRequest,
    response: SetClockRateResponse,
}

impl SetClockRate {
    pub fn new(clock_id: u32, rate_hz: u32, skip_setting_turbo: u32) -> Self {
        SetClockRate {
            request: SetClockRateRequest {
                size: mem::size_of::<SetClockRateRequest>() as u32,
                code: 0,
                tag_id0: TagId::GetClockRate as u32,
                tag_buffer_size0: 12,
                tag_code0: 0,
                clock_id: clock_id,
                rate_hz: rate_hz,
                skip_setting_turbo: skip_setting_turbo,
                end_tag: 0,
            },
        }
    }
}

#[allow(dead_code)]
enum GpioPull {
    Off = 0,
    Down,
    Up,
}

struct Pl011Uart {
    gpio_reg: RegBlock,
    mbox_reg: RegBlock,
    pl011_reg: RegBlock,
}

impl Pl011Uart {
    pub fn init(&self) {
        // Disable UART0
        write_reg(self.pl011_reg, UART0_CR, 0);

        // Turn pull up/down off for pins 14/15 (tx/rx)
        self.gpiosetpull(14, GpioPull::Off);
        self.gpiosetpull(15, GpioPull::Off);

        // Clear interrupts
        write_reg(self.pl011_reg, UART0_ICR, 0x7ff);

        // Read status register until full flag not set
        while (read_reg(self.mbox_reg, MBOX_STATUS) & MBOX_FULL) != 0 {}

        // Set the uart clock rate to 3MHz
        let uart_clock_rate_hz = 3_000_000;
        let set_clock_rate_req = SetClockRate::new(2, uart_clock_rate_hz, 0);
        let channel = ChannelId::ArmToVc;

        // Write the request address combined with the channel to the write register
        let uart_mbox_u32 = ptr::addr_of!(set_clock_rate_req) as u32;
        let r = (uart_mbox_u32 & !0xF) | (channel as u32);
        write_reg(self.mbox_reg, MBOX_WRITE, r);

        // Wait for response
        loop {
            while (read_reg(self.mbox_reg, MBOX_STATUS) & MBOX_EMPTY) != 0 {}
            let response = read_reg(self.mbox_reg, MBOX_READ);
            if response == r {
                break;
            }
        }

        // Set the baud rate via the integer and fractional baud rate regs
        let baud_rate = 115200;
        let baud_rate_divisor = (uart_clock_rate_hz as f32) / ((16 * baud_rate) as f32);
        let int_brd = baud_rate_divisor as u32;
        let frac_brd = (((baud_rate_divisor - (int_brd as f32)) * 64.0) + 0.5) as u32;
        write_reg(self.pl011_reg, UART0_IBRD, int_brd);
        write_reg(self.pl011_reg, UART0_FBRD, frac_brd);

        // Enable FIFOs (tx and rx), 8 bit
        write_reg(self.pl011_reg, UART0_LCRH, 0x70);

        // Mask all interrupts
        write_reg(self.pl011_reg, UART0_IMSC, 0x7f2);

        // Enable UART0, receive only
        write_reg(self.pl011_reg, UART0_CR, 0x81);
    }

    fn gpiosetpull(&self, pin: u32, pull: GpioPull) {
        // The GPIO pull up/down bits are spread across consecutive registers GPPUDCLK0 to GPPUDCLK1
        // GPPUDCLK0: pins  0-31
        // GPPUDCLK1: pins 32-53
        let reg_offset = pin as u64 / 32;
        // Number of bits to shift pull, in order to affect the required pin (just 1 bit)
        let pud_bit = 1 << (pin % 32);
        // Which GPPUDCLK register to use
        let gppudclk_reg = GPPUDCLK0 + reg_offset * 4;

        // You can't read the GPPUD registers, so to set the state we first set the PUD value we want...
        write_reg(self.gpio_reg, GPPUD, pull as u32);
        // ...wait 150 cycles for it to set
        delay(150);
        // ...set the appropriate PUD bit
        write_reg(self.gpio_reg, gppudclk_reg, pud_bit);
        // ...wait 150 cycles for it to set
        delay(150);
        // ...clear up
        write_reg(self.gpio_reg, GPPUD, 0);
        write_reg(self.gpio_reg, gppudclk_reg, 0);
    }
}

impl Uart for Pl011Uart {
    fn putb(&self, b: u8) {
        // Wait for UART to become ready to transmit.
        while read_reg(self.pl011_reg, UART0_FR) & (1 << 5) != 0 {}
        write_reg(self.pl011_reg, UART0_DR, b as u32);
    }
}

pub fn init(dt: &DeviceTree) {
    // TODO use aliases?
    let gpio_reg = dt
        .find_compatible("brcm,bcm2835-gpio")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
        .unwrap();

    let mbox_reg = dt
        .find_compatible("brcm,bcm2835-mbox")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
        .unwrap();

    // Find a compatible pl011 uart
    let pl011_reg = dt
        .find_compatible("arm,pl011")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
        .unwrap();

    Console::new(|| {
        let uart = Pl011Uart { gpio_reg, pl011_reg, mbox_reg };
        uart.init();

        static mut UART: MaybeUninit<Pl011Uart> = MaybeUninit::uninit();
        unsafe {
            UART.write(uart);
            UART.assume_init_mut()
        }
    });
}
