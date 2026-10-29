//! The console: the UART the architecture registers, and print, iprint and
//! putstrn, which write to it.  The console device, #c, is devcons.

use core::time::Duration;
use core::{fmt, hint};

use crate::clock;
use crate::irq;
use crate::mcslock::{Lock, LockGuard, LockNode};

const fn ctrl(b: u8) -> u8 {
    b - b'@'
}

const BACKSPACE: u8 = ctrl(b'H');
#[expect(dead_code)]
const DELETE: u8 = 0x7F;
#[expect(dead_code)]
const CTLD: u8 = ctrl(b'D');
#[expect(dead_code)]
const CTLP: u8 = ctrl(b'P');
#[expect(dead_code)]
const CTLU: u8 = ctrl(b'U');

/// Sync, since iprint on one CPU and print on another can use it at once.
pub trait Uart: Sync {
    fn putb(&self, b: u8);
}

/// Serializes print's output.
static PRINTLOCK: Lock<()> = Lock::new("print", ());

/// Serializes iprint's output, as far as a bounded wait for it allows.
static IPRINTLOCK: Lock<()> = Lock::new("iprint", ());

/// The UART, in a lock of its own so that iprint can reach it without
/// PRINTLOCK.  It's taken with ilock, held only to copy the reference, and
/// always the last lock taken.
static UART: Lock<Option<&'static dyn Uart>> = Lock::new("uart", None);

fn uart() -> Option<&'static dyn Uart> {
    let node = LockNode::new();
    *UART.ilock(&node)
}

/// Console is what should be used in almost all cases, as it ensures threadsafe
/// use of the console.
pub struct Console;

impl Console {
    pub fn set_uart(uart: &'static dyn Uart) {
        let node = LockNode::new();
        *UART.ilock(&node) = Some(uart);
    }

    pub fn putstr(&mut self, s: &str) {
        putstrn(s.as_bytes());
    }
}

/// Write `s` to the console, as Plan 9's putstrn.  It's bytes rather than a
/// str, since user output needn't be UTF-8.
pub fn putstrn(s: &[u8]) {
    // PRINTLOCK is taken with IRQs unmasked, so a handler that printed while
    // its CPU held it would deadlock.  Handlers use iprint.
    assert!(!irq::in_interrupt(), "print in interrupt context");
    let node = LockNode::new();
    let _print = PRINTLOCK.lock(&node);
    if let Some(uart) = uart() {
        puts(uart, s);
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

/// Print with IRQs masked, without the console lock, straight to the UART, as
/// Plan 9's iprint: for interrupt context, panics, and debugging the console.
/// IRQs stay masked for the whole output, about 87 µs a character at 115200
/// baud, so an interrupt handler should keep it to a line.
pub fn iprint(args: fmt::Arguments) {
    irq::masked(|| {
        let node = LockNode::new();
        let _iprint = iprintcanlock(&node);
        if let Some(uart) = uart() {
            let _ = fmt::write(&mut Writer(uart), args);
        }
    });
}

/// Take IPRINTLOCK, as Plan 9's iprintcanlock, giving up after IPRINT_TIMEOUT
/// so that a panic part way through an iprint can still report.  Without a
/// clock it tries once: only one CPU runs then, so only a recursion on this
/// CPU could be holding the lock.
fn iprintcanlock(node: &LockNode) -> Option<LockGuard<'_, ()>> {
    const IPRINT_TIMEOUT: Duration = Duration::from_millis(100);

    let (Some(now), Some(hz)) = (clock::now(), clock::frequency()) else {
        return IPRINTLOCK.try_lock(node);
    };

    let deadline = now.saturating_add(clock::to_counts(IPRINT_TIMEOUT, hz));
    loop {
        if let Some(guard) = IPRINTLOCK.try_lock(node) {
            return Some(guard);
        }
        if clock::now().is_none_or(|t| t >= deadline) {
            return None;
        }
        hint::spin_loop();
    }
}

/// Writes formatted output straight to a UART.
struct Writer(&'static dyn Uart);

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        puts(self.0, s.as_bytes());
        Ok(())
    }
}

#[macro_export]
macro_rules! iprintln {
    () => ($crate::iprint!("\n"));
    ($($arg:tt)*) => ($crate::iprint!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! iprint {
    ($($args:tt)*) => {{
        $crate::console::iprint(format_args!($($args)*))
    }};
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! print {
    ($($args:tt)*) => {{
        $crate::console::print(format_args!($($args)*))
    }};
}

fn puts(uart: &dyn Uart, s: &[u8]) {
    for &b in s {
        if b == b'\n' {
            uart.putb(b'\r');
        } else if b == BACKSPACE {
            uart.putb(b);
            uart.putb(b' ');
        }
        uart.putb(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{cons_masked, cons_output, fake_cons};

    #[test]
    fn iprint_without_printlock() {
        fake_cons();
        let node = LockNode::new();
        let _print = PRINTLOCK.lock(&node);
        crate::iprintln!("held {}", 1);
        assert_eq!(cons_output(), b"held 1\r\n");
    }

    #[test]
    fn iprint_without_a_clock_tries_once() {
        fake_cons();
        let node = LockNode::new();
        let _iprint = IPRINTLOCK.lock(&node);
        crate::iprint!("stuck");
        assert_eq!(cons_output(), b"stuck");
    }

    #[test]
    fn iprint_gives_up_waiting_after_iprint_wait() {
        thread_local! {
            static NOW: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
        }
        // A 1 kHz clock that advances a tick each time it's read.
        static FAKE: clock::Ops = clock::Ops {
            now: || NOW.replace(NOW.get() + 1),
            frequency: || 1000,
            set_timer: |_| {},
        };
        clock::init(&FAKE);
        fake_cons();
        let node = LockNode::new();
        let _iprint = IPRINTLOCK.lock(&node);
        crate::iprint!("stuck");
        assert_eq!(cons_output(), b"stuck");
        // It waited out 100 ms, 100 ticks at 1 kHz, and gave up soon after.
        let ticks = NOW.get();
        assert!((100..=102).contains(&ticks), "gave up after {ticks} ticks");
    }

    #[test]
    fn iprint_masks_irqs() {
        irq::init(&irq::tests::FAKE);
        fake_cons();
        crate::iprint!("x");
        assert_eq!(cons_output(), b"x");
        assert_eq!(cons_masked(), [true]);
        crate::print!("y");
        assert_eq!(cons_masked(), [false]);
    }

    #[test]
    #[should_panic(expected = "print in interrupt context")]
    fn print_in_interrupt_panics() {
        irq::init(&irq::tests::FAKE);
        irq::enter_interrupt();
        crate::print!("x");
    }
}
