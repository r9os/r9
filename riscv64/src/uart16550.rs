use core::convert::TryInto;
use core::fmt::Error;
use core::fmt::Write;

use port::devcons::Uart;
use port::println;

pub struct Uart16550 {
    base: *mut u8,
}

impl Write for Uart16550 {
    fn write_str(&mut self, out: &str) -> Result<(), Error> {
        for c in out.bytes() {
            self.put(c);
        }
        Ok(())
    }
}

impl Uart for Uart16550 {
    fn putb(&self, b: u8) {
        let ptr = self.base;
        unsafe {
            ptr.add(0).write_volatile(b);
        }
    }
}

impl Uart16550 {
    pub fn new(addr: usize) -> Self {
        Uart16550 { base: addr as *mut u8 }
    }

    // see also https://www.lookrs232.com/rs232/dlab.htm
    pub fn init(&mut self, baud: u32) {
        let ptr = self.base;
        let divisor: u16 = (2_227_900 / (baud * 16)) as u16; // set baud rate
        let divisor_least: u8 = (divisor & 0xff).try_into().unwrap();
        let divisor_most: u8 = (divisor >> 8).try_into().unwrap();
        let word_length = 3;
        unsafe {
            // set word length
            ptr.add(3).write_volatile(word_length);
            // enable FIFO
            ptr.add(2).write_volatile(1);
            // enable receiver interrupts
            ptr.add(1).write_volatile(1);
            // access DLAB (Divisor Latch Access Bit)
            ptr.add(3).write_volatile(word_length | 1 << 7);
            // divisor low byte
            ptr.add(0).write_volatile(divisor_least);
            // divisor high byte
            ptr.add(1).write_volatile(divisor_most);
            // close DLAB
            ptr.add(3).write_volatile(word_length);
        }
    }

    pub fn put(&mut self, c: u8) {
        let ptr = self.base;
        unsafe {
            ptr.add(0).write_volatile(c);
        }
    }

    pub fn get(&mut self) -> Option<u8> {
        let ptr = self.base;
        unsafe {
            if ptr.add(5).read_volatile() & 1 == 0 {
                None
            } else {
                Some(ptr.add(0).read_volatile())
            }
        }
    }
}
