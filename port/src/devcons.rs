use crate::mcslock::{Lock, LockNode};
use core::fmt;

const fn ctrl(b: u8) -> u8 {
    b - b'@'
}

#[allow(dead_code)]
const BACKSPACE: u8 = ctrl(b'H');
#[allow(dead_code)]
const DELETE: u8 = 0x7F;
#[allow(dead_code)]
const CTLD: u8 = ctrl(b'D');
#[allow(dead_code)]
const CTLP: u8 = ctrl(b'P');
#[allow(dead_code)]
const CTLU: u8 = ctrl(b'U');

pub trait Uart {
    fn putb(&self, b: u8);
}

pub struct Console<T: Uart> {
    uart: T,
}

impl<T: Uart> Console<T> {
    pub fn new(uart: T) -> Self {
        Self { uart: uart }
    }

    pub fn putb(&mut self, b: u8) {
        if b == b'\n' {
            self.uart.putb(b'\r');
        } else if b == BACKSPACE {
            self.uart.putb(b);
            self.uart.putb(b' ');
        }
        self.uart.putb(b);
    }

    pub fn putstr(&mut self, s: &str) {
        static LOCK: Lock<()> = Lock::new("println", ());
        // XXX: Just for testing.
        static mut NODE: LockNode = LockNode::new();
        let _guard = LOCK.lock(unsafe { &NODE });
        for b in s.bytes() {
            self.putb(b);
        }
    }
}

impl<T: Uart> fmt::Write for Console<T> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.putstr(s);
        Ok(())
    }
}
