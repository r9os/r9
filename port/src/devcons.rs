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

pub struct Console;

static CONS: Lock<Option<&'static mut dyn Uart>> = Lock::new("cons", None);

impl Console {
    pub fn new<F>(uart_fn: F) -> Self
    where
        F: FnOnce() -> &'static mut dyn Uart,
    {
        static mut NODE: LockNode = LockNode::new();
        let mut cons = CONS.lock(unsafe { &NODE });
        *cons = Some(uart_fn());
        Self
    }

    pub fn putb(&mut self, uart: &mut dyn Uart, b: u8) {
        if b == b'\n' {
            uart.putb(b'\r');
        } else if b == BACKSPACE {
            uart.putb(b);
            uart.putb(b' ');
        }
        uart.putb(b);
    }

    pub fn putstr(&mut self, s: &str) {
        // XXX: Just for testing.
        static mut NODE: LockNode = LockNode::new();
        let mut uart = CONS.lock(unsafe { &NODE });
        for b in s.bytes() {
            self.putb(uart.as_deref_mut().unwrap(), b);
        }
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.putstr(s);
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    // XXX: Just for testing.
    use fmt::Write;
    let mut cons: Console = Console {};
    cons.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! print {
    ($($args:tt)*) => {{
        $crate::devcons::print(format_args!($($args)*))
    }};
}
