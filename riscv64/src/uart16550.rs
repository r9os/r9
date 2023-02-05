use core::convert::TryInto;
use core::fmt::Error;
use core::fmt::Write;

use port::devcons::Uart;
use port::fdt::RegBlock;

pub struct Uart16550 {
    pub ns16550a_reg: RegBlock,
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
        let ptr = self.ns16550a_reg.addr as *mut u8;
        unsafe {
            ptr.add(0).write_volatile(b);
        }
    }
}

impl Uart16550 {
    pub fn new(ns16550a_reg: RegBlock) -> Self {
        Uart16550 { ns16550a_reg }
    }

    pub fn init(&mut self, baud: u32) {
        let ptr = self.ns16550a_reg.addr as *mut u8;
        unsafe {
            let lcr = 3; // word length
            ptr.add(3).write_volatile(lcr); // set word length
            ptr.add(2).write_volatile(1); // enable FIFO
            ptr.add(1).write_volatile(1); // enable receiver interrupts
            let divisor: u16 = (2_227_900 / (baud * 16)) as u16; // set baud rate
            let divisor_least: u8 = (divisor & 0xff).try_into().unwrap();
            let divisor_most: u8 = (divisor >> 8).try_into().unwrap();
            ptr.add(3).write_volatile(lcr | 1 << 7); // access DLAB
            ptr.add(0).write_volatile(divisor_least); // DLL
            ptr.add(1).write_volatile(divisor_most); // DLM
            ptr.add(3).write_volatile(lcr); // close DLAB
        }
    }

    pub fn put(&mut self, c: u8) {
        let ptr = self.ns16550a_reg.addr as *mut u8;
        unsafe {
            ptr.add(0).write_volatile(c);
        }
    }

    pub fn get(&mut self) -> Option<u8> {
        let ptr = self.ns16550a_reg.addr as *mut u8;
        unsafe {
            if ptr.add(5).read_volatile() & 1 == 0 {
                None
            } else {
                Some(ptr.add(0).read_volatile())
            }
        }
    }
}
