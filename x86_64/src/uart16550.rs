//! Simple UART driver to get started.

pub fn putb(port: u16, b: u8) {
    unsafe {
        crate::pio::outb(port, b);
    }
}
