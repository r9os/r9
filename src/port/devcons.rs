// Racy to start.

use crate::port::mcslock::{Lock, LockNode};
use core::fmt;

const fn ctrl(b: u8) -> u8 {
    b - b'@'
}

const BACKSPACE: u8 = ctrl(b'H');
const DELETE: u8 = 0x7F;
const CTLD: u8 = ctrl(b'D');
const CTLP: u8 = ctrl(b'P');
const CTLU: u8 = ctrl(b'U');

pub struct Console(pub u16);

impl Console {
    fn putb(&mut self, b: u8) {
        fn uartputb(port: u16, b: u8) {
            crate::x86_64::uart16550::putb(port, b);
        }
        if b == b'\n' {
            uartputb(self.0, b'\r');
        } else if b == BACKSPACE {
            uartputb(self.0, b);
            uartputb(self.0, b' ');
        }
        uartputb(self.0, b);
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

pub fn print(args: fmt::Arguments) {
    use core::fmt::Write;
    let mut cons = crate::port::devcons::Console(0x3f8);
    cons.write_fmt(args).unwrap();
}

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.putstr(s);
        Ok(())
    }
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! print {
    ($($args:tt)*) => {{
        $crate::port::devcons::print(format_args!($($args)*))
    }};
}
