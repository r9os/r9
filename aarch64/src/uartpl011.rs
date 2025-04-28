use crate::deviceutil::map_device_register;
use crate::io::{GpioPull, delay, read_reg, write_reg};
use crate::registers::{
    GPPUD, GPPUDCLK0, UART0_CR, UART0_DR, UART0_FBRD, UART0_FR, UART0_IBRD, UART0_ICR, UART0_IMSC,
    UART0_LCRH,
};
use crate::{mailbox, vm};
use port::Result;
use port::devcons::Uart;
use port::fdt::DeviceTree;
use port::mem::{PhysRange, VirtRange};

#[cfg(not(test))]
use port::println;

#[allow(dead_code)]
pub struct Pl011Uart {
    gpio_virtrange: VirtRange,
    pl011_virtrange: VirtRange,
}

/// PL011 is the default in qemu (UART0), but a bit fiddly to use on a real
/// Raspberry Pi board, as it needs additional configuration in the config
/// and EEPROM (rpi4) to assign to the serial GPIO pins.
#[allow(dead_code)]
impl Pl011Uart {
    pub fn new(dt: &DeviceTree) -> Result<Pl011Uart> {
        let gpio_physrange = Self::find_gpio_physrange(dt)?;
        let gpio_virtrange = match map_device_register("gpio", gpio_physrange, vm::PageSize::Page4K)
        {
            Ok(gpio_virtrange) => gpio_virtrange,
            Err(msg) => {
                println!("can't map gpio {:?}", msg);
                return Err("can't create pl011");
            }
        };

        let pl011_physrange = Self::find_pl011_physrange(dt)?;
        let pl011_virtrange =
            match map_device_register("pl011", pl011_physrange, vm::PageSize::Page4K) {
                Ok(pl011_virtrange) => pl011_virtrange,
                Err(msg) => {
                    println!("can't map pl011 {:?}", msg);
                    return Err("can't create pl011");
                }
            };

        Ok(Pl011Uart { gpio_virtrange, pl011_virtrange })
    }

    fn find_gpio_physrange(dt: &DeviceTree) -> Result<PhysRange> {
        dt.find_compatible("brcm,bcm2835-gpio")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .ok_or("can't find gpio")
    }

    fn find_pl011_physrange(dt: &DeviceTree) -> Result<PhysRange> {
        dt.find_compatible("arm,pl011")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .ok_or("can't find pl011")
    }

    pub fn init(&self) {
        // Disable UART0
        write_reg(&self.pl011_virtrange, UART0_CR, 0);

        // Turn pull up/down off for pins 14/15 (tx/rx)
        self.gpiosetpull(14, GpioPull::Off);
        self.gpiosetpull(15, GpioPull::Off);

        // Clear interrupts
        write_reg(&self.pl011_virtrange, UART0_ICR, 0x7ff);

        // Set the uart clock rate to 3MHz
        let uart_clock_rate_hz = 3_000_000;
        mailbox::set_clock_rate(2, uart_clock_rate_hz, 0);

        // Set the baud rate via the integer and fractional baud rate regs
        let baud_rate = 115200;
        let baud_rate_divisor = (uart_clock_rate_hz as f32) / ((16 * baud_rate) as f32);
        let int_brd = baud_rate_divisor as u32;
        let frac_brd = (((baud_rate_divisor - (int_brd as f32)) * 64.0) + 0.5) as u32;
        write_reg(&self.pl011_virtrange, UART0_IBRD, int_brd);
        write_reg(&self.pl011_virtrange, UART0_FBRD, frac_brd);

        // Enable FIFOs (tx and rx), 8 bit
        write_reg(&self.pl011_virtrange, UART0_LCRH, 0x70);

        // Mask all interrupts
        write_reg(&self.pl011_virtrange, UART0_IMSC, 0x7f2);

        // Enable UART0, receive only
        write_reg(&self.pl011_virtrange, UART0_CR, 0x81);
    }

    fn gpiosetpull(&self, pin: u32, pull: GpioPull) {
        // The GPIO pull up/down bits are spread across consecutive registers GPPUDCLK0 to GPPUDCLK1
        // GPPUDCLK0: pins  0-31
        // GPPUDCLK1: pins 32-53
        let reg_offset = pin as usize / 32;
        // Number of bits to shift pull, in order to affect the required pin (just 1 bit)
        let pud_bit = 1 << (pin % 32);
        // Which GPPUDCLK register to use
        let gppudclk_reg = GPPUDCLK0 + reg_offset * 4;

        // You can't read the GPPUD registers, so to set the state we first set the PUD value we want...
        write_reg(&self.pl011_virtrange, GPPUD, pull as u32);
        // ...wait 150 cycles for it to set
        delay(150);
        // ...set the appropriate PUD bit
        write_reg(&self.pl011_virtrange, gppudclk_reg, pud_bit);
        // ...wait 150 cycles for it to set
        delay(150);
        // ...clear up
        write_reg(&self.pl011_virtrange, GPPUD, 0);
        write_reg(&self.pl011_virtrange, gppudclk_reg, 0);
    }
}

impl Uart for Pl011Uart {
    fn putb(&self, b: u8) {
        // Wait for UART to become ready to transmit.
        while read_reg(&self.pl011_virtrange, UART0_FR) & (1 << 5) != 0 {}
        write_reg(&self.pl011_virtrange, UART0_DR, b as u32);
    }
}
