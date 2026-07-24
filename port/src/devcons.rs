use crate::Result;
use crate::irq::IrqGuard;
use crate::mcslock::{Lock, LockNode};
use core::fmt;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

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

static CONS: Lock<Option<&'static dyn Uart>> = Lock::new("cons", None);

/// Console is what should be used in almost all cases, as it ensures threadsafe
/// use of the console.
pub struct Console;

impl Console {
    pub fn set_uart<F>(uart_fn: F)
    where
        F: FnOnce() -> Result<&'static dyn Uart>,
    {
        let node = LockNode::new();
        let mut cons = CONS.lock(&node);
        *cons = uart_fn().ok();
    }

    pub fn putstr(&mut self, s: &str) {
        // XXX: Just for testing.

        // The console lock is thread-context only; interrupt context
        // must use iprint, which bypasses it.
        debug_assert!(!crate::irq::in_interrupt(), "println in interrupt context; use iprintln");
        let node = LockNode::new();
        let uart_guard = CONS.lock(&node);
        if let Some(uart) = *uart_guard {
            for b in s.bytes() {
                putb(uart, b);
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

fn putb(uart: &dyn Uart, b: u8) {
    if b == b'\n' {
        uart.putb(b'\r');
    } else if b == BACKSPACE {
        uart.putb(b);
        uart.putb(b' ');
    }
    uart.putb(b);
}

// iprint: the interrupt- and panic-safe print, in the tradition of
// Plan 9's iprint.  It masks IRQs, takes only a best-effort interlock,
// and writes polled bytes directly to the hardware, bypassing the
// console lock — so it works in interrupt context, in panic, and while
// debugging the console or locks themselves.  Output may interleave
// with a concurrent print; that is the accepted price of never
// blocking.

/// Direct console byte writer, registered by arch code at boot (same
/// pattern as `irq::set_ops`).  Must be polled and lock-free: it is
/// called with no locks held from any context.
pub struct IprintOps {
    pub putb: fn(u8),
}

static IPRINT_OPS: AtomicPtr<IprintOps> = AtomicPtr::new(ptr::null_mut());

/// Register the direct console writer.  Call at boot once the console
/// hardware is initialised; until then iprint output is dropped.
pub fn set_iprint_ops(ops: &'static IprintOps) {
    IPRINT_OPS.store(ops as *const IprintOps as *mut IprintOps, Ordering::Release);
}

fn iprint_ops() -> Option<&'static IprintOps> {
    unsafe { IPRINT_OPS.load(Ordering::Acquire).as_ref() }
}

/// Best-effort interlock so concurrent iprints don't interleave.
/// Never required for correctness — see `iprint_trylock`.
static IPRINT_LOCK: AtomicBool = AtomicBool::new(false);

/// Try to take the interlock, giving up after a bounded spin: if
/// another core holds it too long, print anyway — interleaved output
/// beats a silent core.  A same-core holder is impossible, as the lock
/// is only ever held with IRQs masked.
fn iprint_trylock() -> bool {
    for _ in 0..1_000_000 {
        if IPRINT_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

struct IprintWriter {
    putb: fn(u8),
}

impl fmt::Write for IprintWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                (self.putb)(b'\r');
            }
            (self.putb)(b);
        }
        Ok(())
    }
}

pub fn iprint(args: fmt::Arguments) {
    let _irq = IrqGuard::new();
    let Some(ops) = iprint_ops() else {
        return;
    };
    let locked = iprint_trylock();
    let _ = fmt::Write::write_fmt(&mut IprintWriter { putb: ops.putb }, args);
    if locked {
        IPRINT_LOCK.store(false, Ordering::Release);
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
        $crate::devcons::iprint(format_args!($($args)*))
    }};
}
