use port::Result;
use port::console::Uart;
use port::fdt::DeviceTree;
use port::mem::{PhysRange, VirtRange};

use crate::deviceutil::map_device_register;
use crate::io::{delay, read_reg, write_or_reg, write_reg};
use crate::registers::{GPFSEL1, GPPUD, GPPUDCLK0};
use crate::vm;

#[cfg(not(test))]
use port::println;

// AUX registers, offset from aux_reg
pub const AUX_IRQ: usize = 0x00; // AUX interrupt status (Mini Uart, SPIs)
pub const AUX_ENABLE: usize = 0x04; // AUX enable register (Mini Uart, SPIs)

// UART1 registers, offset from miniuart_reg
pub const AUX_MU_IO: usize = 0x00; // AUX IO data register
pub const AUX_MU_IER: usize = 0x04; // Mini Uart interrupt enable register
pub const AUX_MU_IIR: usize = 0x08; // Mini Uart interrupt identify register
pub const AUX_MU_LCR: usize = 0x0c; // Mini Uart line control register
pub const AUX_MU_MCR: usize = 0x10; // Mini Uart line control register
pub const AUX_MU_LSR: usize = 0x14; // Mini Uart line status register
pub const AUX_MU_CNTL: usize = 0x20; // Mini Uart control register
pub const AUX_MU_BAUD: usize = 0x28; // Mini Uart baudrate register

/// MiniUart is assigned to UART1 on the Raspberry Pi.  It is easier to use with
/// real hardware, as it requires no additional configuration.  Conversely, it's
/// harded to use with QEMU, as it can't be used with the `nographic` switch.
pub struct MiniUart {
    pub gpio_virtrange: VirtRange,
    pub aux_virtrange: VirtRange,
    pub miniuart_virtrange: VirtRange,
}

#[expect(dead_code)]
impl MiniUart {
    /// Create MiniUart assuming the required register have already been mapped.
    /// This is intended for use only at early startup, *before* the full VM code has been set up,
    /// and should be replaced by a MiniUart with specifically mapped ranges *after* the VM has
    /// been set up.
    pub fn new_assuming_mapped_mmio(dt: &DeviceTree, mmio_virt_offset: usize) -> Result<MiniUart> {
        let gpio_virtrange = Self::find_gpio_physrange(dt)
            .map(|pr| VirtRange::from_physrange(&pr, mmio_virt_offset))?;

        let aux_virtrange = Self::find_aux_physrange(dt)
            .map(|pr| VirtRange::from_physrange(&pr, mmio_virt_offset))?;

        let miniuart_virtrange = Self::find_miniuart_physrange(dt)
            .map(|pr| VirtRange::from_physrange(&pr, mmio_virt_offset))?;

        Ok(MiniUart { gpio_virtrange, aux_virtrange, miniuart_virtrange })
    }

    pub fn new_with_map_ranges(dt: &DeviceTree) -> Result<MiniUart> {
        let gpio_physrange = Self::find_gpio_physrange(dt)?;
        let gpio_virtrange = match map_device_register("gpio", gpio_physrange, vm::PageSize::Page4K)
        {
            Ok(gpio_virtrange) => gpio_virtrange,
            Err(msg) => {
                println!("can't map gpio {:?}", msg);
                return Err("can't create miniuart");
            }
        };

        let aux_physrange = Self::find_aux_physrange(dt)?;
        let aux_virtrange = match map_device_register("aux", aux_physrange, vm::PageSize::Page4K) {
            Ok(aux_virtrange) => aux_virtrange,
            Err(msg) => {
                println!("can't map aux {:?}", msg);
                return Err("can't create miniuart");
            }
        };

        let miniuart_physrange = Self::find_miniuart_physrange(dt)?;
        let miniuart_virtrange =
            match map_device_register("miniuart", miniuart_physrange, vm::PageSize::Page4K) {
                Ok(aux_virtrange) => aux_virtrange,
                Err(msg) => {
                    println!("can't map miniuart {:?}", msg);
                    return Err("can't create miniuart");
                }
            };

        Ok(MiniUart { gpio_virtrange, aux_virtrange, miniuart_virtrange })
    }

    /// Bcm2835 and bcm2711 are essentially the same for our needs here.
    /// If fdt.rs supported aliases well, we could try to just look up 'gpio'.
    fn find_gpio_physrange(dt: &DeviceTree) -> Result<PhysRange> {
        dt.find_compatible("brcm,bcm2835-gpio")
            .next()
            .or_else(|| dt.find_compatible("brcm,bcm2711-gpio").next())
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .ok_or("can't find gpio")
    }

    /// Find a compatible aux
    fn find_aux_physrange(dt: &DeviceTree) -> Result<PhysRange> {
        dt.find_compatible("brcm,bcm2835-aux")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .ok_or("can't find aux")
    }

    /// Find a compatible miniuart
    fn find_miniuart_physrange(dt: &DeviceTree) -> Result<PhysRange> {
        dt.find_compatible("brcm,bcm2835-aux-uart")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .ok_or("can't find miniuart")
    }

    pub fn init(&self) {
        // Set GPIO pins 14 and 15 to be used for UART1.  This is done by
        // setting the appropriate flags in GPFSEL1 to ALT5, which is
        // represented by the 0b010
        let mut gpfsel1 = read_reg(&self.gpio_virtrange, GPFSEL1);
        gpfsel1 &= !((7 << 12) | (7 << 15));
        gpfsel1 |= (2 << 12) | (2 << 15);
        write_reg(&self.gpio_virtrange, GPFSEL1, gpfsel1);

        write_reg(&self.gpio_virtrange, GPPUD, 0);
        delay(150);
        write_reg(&self.gpio_virtrange, GPPUDCLK0, (1 << 14) | (1 << 15));
        delay(150);
        write_reg(&self.gpio_virtrange, GPPUDCLK0, 0);

        // Enable mini uart - required to write to its registers
        write_or_reg(&self.aux_virtrange, AUX_ENABLE, 1);
        write_reg(&self.miniuart_virtrange, AUX_MU_CNTL, 0);
        // 8-bit
        write_reg(&self.miniuart_virtrange, AUX_MU_LCR, 3);
        write_reg(&self.miniuart_virtrange, AUX_MU_MCR, 0);
        // Disable interrupts
        write_reg(&self.miniuart_virtrange, AUX_MU_IER, 0);
        // Clear receive/transmit FIFOs
        write_reg(&self.miniuart_virtrange, AUX_MU_IIR, 0xc6);

        // We want 115200 baud.  This is calculated as:
        //   system_clock_freq / (8 * (baudrate_reg + 1))
        // For now we're making assumptions about the clock frequency
        // TODO Get the clock freq via the mailbox, and update if it changes.
        // let arm_clock_rate = 500000000.0;
        // let baud_rate_reg = arm_clock_rate / (8.0 * 115200.0) - 1.0;
        //write_reg(self.miniuart_reg, AUX_MU_BAUD, baud_rate_reg as u32);
        write_reg(&self.miniuart_virtrange, AUX_MU_BAUD, 545);

        // Finally enable transmit
        write_reg(&self.miniuart_virtrange, AUX_MU_CNTL, 3);
    }
}

/// The receive FIFO's depth (BCM2711 ARM Peripherals, 2.2).
pub const RX_FIFO: usize = 8;

impl MiniUart {
    /// The mini UART's interrupt: its node's interrupts is a GIC interrupt's
    /// three cells, type, number and flags, and it's an SPI (type 0), whose
    /// INTID is 32 plus its number (IHI 0048B.b, 2.2.1).  It's shared with
    /// the two auxiliary SPIs (BCM2711 ARM Peripherals, 2.1).
    pub fn find_intid(dt: &DeviceTree) -> Result<usize> {
        let node =
            dt.find_compatible("brcm,bcm2835-aux-uart").next().ok_or("can't find miniuart")?;
        let interrupts = dt.property(&node, "interrupts").ok_or("miniuart has no interrupts")?;
        let mut cells = dt.property_value_as_u32_iter(&interrupts);
        match (cells.next(), cells.next()) {
            (Some(0), Some(n)) => Ok(32 + n as usize),
            _ => Err("miniuart's interrupt isn't an SPI"),
        }
    }

    /// Interrupt whenever the receive FIFO holds a byte: AUX_MU_IER_REG's
    /// bit 0.  BCM2711 ARM Peripherals, 2.2.2, Table 6, gives it as bit 1,
    /// repeating the BCM2835 manual's swapped bits, but QEMU's
    /// bcm2835_aux.c (RX_INT is 0x1) and Linux's 8250_bcm2835aux (an 8250's
    /// UART_IER_RDI, 0x01) agree it's bit 0.  The transmit interrupt, bit 1,
    /// stays off: it's asserted whenever the transmit FIFO is empty.
    pub fn enable_receive_interrupt(&self) {
        write_reg(&self.miniuart_virtrange, AUX_MU_IER, 1);
    }

    /// Whether the mini UART's interrupt is pending, rather than an
    /// auxiliary SPI's: AUX_IRQ's bit 0 (BCM2711 ARM Peripherals, 2.1.1,
    /// Table 3).
    pub fn interrupt_pending(&self) -> bool {
        read_reg(&self.aux_virtrange, AUX_IRQ) & 1 != 0
    }

    /// Take what the receive FIFO holds into `buf`, and return how much:
    /// bytes from AUX_MU_IO_REG while AUX_MU_LSR_REG's data ready bit, bit 0,
    /// is set, at most the FIFO's depth, so the work is bounded.  Bytes that
    /// arrived while the FIFO was full are lost, as LSR's receiver overrun
    /// bit, bit 1, says until LSR is read (2.2.2, Table 10), and they're
    /// dropped without a report, as Plan 9's console drops input it has no
    /// room for.  The receive interrupt is level-triggered, and drops once
    /// the FIFO's empty.
    pub fn drain(&self, buf: &mut [u8; RX_FIFO]) -> usize {
        const DATA_READY: u32 = 1 << 0;
        let mut n = 0;
        while n < RX_FIFO && read_reg(&self.miniuart_virtrange, AUX_MU_LSR) & DATA_READY != 0 {
            buf[n] = read_reg(&self.miniuart_virtrange, AUX_MU_IO) as u8;
            n += 1;
        }
        n
    }
}

impl Uart for MiniUart {
    fn putb(&self, b: u8) {
        // Wait for UART to become ready to transmit
        while read_reg(&self.miniuart_virtrange, AUX_MU_LSR) & (1 << 5) == 0 {
            core::hint::spin_loop();
        }
        write_reg(&self.miniuart_virtrange, AUX_MU_IO, b as u32);
    }
}
