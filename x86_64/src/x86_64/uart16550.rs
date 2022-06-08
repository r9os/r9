/// Simple UART driver to get setarted.

pub fn putb(port: u16, b: u8) {
    unsafe {
        crate::x86_64::outb(port, b);
    }
}
