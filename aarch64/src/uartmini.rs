use port::Result;
use port::devcons::Uart;
use port::fdt::DeviceTree;
use port::mem::{PhysRange, VirtRange};

use crate::deviceutil::map_device_register;
use crate::io::{delay, read_reg, write_or_reg, write_reg};
use crate::registers::{
    AUX_ENABLE, AUX_MU_BAUD, AUX_MU_CNTL, AUX_MU_IER, AUX_MU_IIR, AUX_MU_IO, AUX_MU_LCR,
    AUX_MU_LSR, AUX_MU_MCR, GPFSEL1, GPPUD, GPPUDCLK0,
};
use crate::vm;

#[cfg(not(test))]
use port::println;

/// MiniUart is assigned to UART1 on the Raspberry Pi.  It is easier to use with
/// real hardware, as it requires no additional configuration.  Conversely, it's
/// harded to use with QEMU, as it can't be used with the `nographic` switch.
pub struct MiniUart {
    pub gpio_virtrange: VirtRange,
    pub aux_virtrange: VirtRange,
    pub miniuart_virtrange: VirtRange,
}

#[allow(dead_code)]
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

impl Uart for MiniUart {
    fn putb(&self, b: u8) {
        // Wait for UART to become ready to transmit
        while read_reg(&self.miniuart_virtrange, AUX_MU_LSR) & (1 << 5) == 0 {
            core::hint::spin_loop();
        }
        write_reg(&self.miniuart_virtrange, AUX_MU_IO, b as u32);
    }
}
