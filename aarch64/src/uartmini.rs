use port::devcons::Uart;
use port::fdt::{DeviceTree, RegBlock};

use crate::io::{delay, read_reg, write_or_reg, write_reg};
use crate::registers::{
    AUX_ENABLE, AUX_MU_BAUD, AUX_MU_CNTL, AUX_MU_IER, AUX_MU_IIR, AUX_MU_IO, AUX_MU_LCR,
    AUX_MU_LSR, AUX_MU_MCR, GPFSEL1, GPPUD, GPPUDCLK0,
};

/// MiniUart is assigned to UART1 on the Raspberry Pi.  It is easier to use with
/// real hardware, as it requires no additional configuration.  Conversely, it's
/// harded to use with QEMU, as it can't be used with the `nographic` switch.
pub struct MiniUart {
    gpio_reg: RegBlock,
    aux_reg: RegBlock,
    miniuart_reg: RegBlock,
}

impl MiniUart {
    pub fn new(dt: &DeviceTree) -> MiniUart {
        // TODO use aliases?
        let gpio_reg = dt
            .find_compatible("brcm,bcm2835-gpio")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .unwrap();

        // Find a compatible aux
        let aux_reg = dt
            .find_compatible("brcm,bcm2835-aux")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .unwrap();

        // Find a compatible miniuart
        let miniuart_reg = dt
            .find_compatible("brcm,bcm2835-aux-uart")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .unwrap();

        MiniUart { gpio_reg, aux_reg, miniuart_reg }
    }

    pub fn init(&self) {
        // Set GPIO pins 14 and 15 to be used for UART1.  This is done by
        // setting the appropriate flags in GPFSEL1 to ALT5, which is
        // represented by the 0b010
        let mut gpfsel1 = read_reg(self.gpio_reg, GPFSEL1);
        gpfsel1 &= !((7 << 12) | (7 << 15));
        gpfsel1 |= (2 << 12) | (2 << 15);
        write_reg(self.gpio_reg, GPFSEL1, gpfsel1);

        write_reg(self.gpio_reg, GPPUD, 0);
        delay(150);
        write_reg(self.gpio_reg, GPPUDCLK0, (1 << 14) | (1 << 15));
        delay(150);
        write_reg(self.gpio_reg, GPPUDCLK0, 0);

        // Enable mini uart - required to write to its registers
        write_or_reg(self.aux_reg, AUX_ENABLE, 1);
        write_reg(self.miniuart_reg, AUX_MU_CNTL, 0);
        // 8-bit
        write_reg(self.miniuart_reg, AUX_MU_LCR, 3);
        write_reg(self.miniuart_reg, AUX_MU_MCR, 0);
        // Disable interrupts
        write_reg(self.miniuart_reg, AUX_MU_IER, 0);
        // Clear receive/transmit FIFOs
        write_reg(self.miniuart_reg, AUX_MU_IIR, 0xc6);

        // We want 115200 baud.  This is calculated as:
        //   system_clock_freq / (8 * (baudrate_reg + 1))
        // For now we're making assumptions about the clock frequency
        // TODO Get the clock freq via the mailbox, and update if it changes.
        // let arm_clock_rate = 500000000.0;
        // let baud_rate_reg = arm_clock_rate / (8.0 * 115200.0) + 1.0;
        //write_reg(self.miniuart_reg, AUX_MU_BAUD, baud_rate_reg as u32);
        write_reg(self.miniuart_reg, AUX_MU_BAUD, 270);

        // Finally enable transmit
        write_reg(self.miniuart_reg, AUX_MU_CNTL, 3);
    }
}

impl Uart for MiniUart {
    fn putb(&self, b: u8) {
        // Wait for UART to become ready to transmit
        while read_reg(self.miniuart_reg, AUX_MU_LSR) & (1 << 5) == 0 {
            core::hint::spin_loop();
        }
        write_reg(self.miniuart_reg, AUX_MU_IO, b as u32);
    }
}
