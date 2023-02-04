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

static mut EARLY_CONS: Option<&'static mut dyn Uart> = None;
static CONS: Lock<Option<&'static mut dyn Uart>> = Lock::new("cons", None);

pub struct Console;

impl Console {
    /// Create a locking console.  Assumes at this point we can use atomics.
    pub fn new<F>(uart_fn: F) -> Self
    where
        F: FnOnce() -> &'static mut dyn Uart,
    {
        static mut NODE: LockNode = LockNode::new();
        let mut cons = CONS.lock(unsafe { &NODE });
        *cons = Some(uart_fn());
        Self
    }

    /// Create an early, non-locking console.  Assumes at this point we cannot use atomics.
    /// Once atomics can be used safely, drop_early_console should be called so we switch
    /// to the locking console.
    pub fn new_early<F>(uart_fn: F) -> Self
    where
        F: FnOnce() -> &'static mut dyn Uart,
    {
        unsafe { EARLY_CONS.replace(uart_fn()) };
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

        if let Some(uart) = unsafe { &mut EARLY_CONS } {
            for b in s.bytes() {
                self.putb(*uart, b);
            }
        } else {
            static mut NODE: LockNode = LockNode::new();
            let mut uart = CONS.lock(unsafe { &NODE });
            for b in s.bytes() {
                self.putb(uart.as_deref_mut().unwrap(), b);
            }
        }
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.putstr(s);
        Ok(())
    }
}

pub fn drop_early_console() {
    static mut NODE: LockNode = LockNode::new();
    let mut cons = CONS.lock(unsafe { &NODE });
    let earlycons = unsafe { EARLY_CONS.take() };
    *cons = earlycons;
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
