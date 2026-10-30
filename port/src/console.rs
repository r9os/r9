//! The console: the UART the architecture registers, print, iprint and
//! putstrn, which write to it, and its input, which the UART's receive
//! interrupt puts and a reader reads a line at a time.  The console device,
//! #c, is devcons.

use core::time::Duration;
use core::{fmt, hint};

use crate::clock;
use crate::dat::{QLock, Rendez};
use crate::irq;
use crate::mcslock::{Lock, LockGuard, LockNode};

const fn ctrl(b: u8) -> u8 {
    b - b'@'
}

const BACKSPACE: u8 = ctrl(b'H');
const CTLD: u8 = ctrl(b'D');
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

/// Make `uart` the console's, as each architecture does as it boots.
pub fn set_uart(uart: &'static dyn Uart) {
    let node = LockNode::new();
    *UART.ilock(&node) = Some(uart);
}

/// Write `s` to the console, as Plan 9's putstrn.  It's bytes rather than a
/// str, since user output needn't be UTF-8.
pub fn putstrn(s: &[u8]) {
    with_printlock(|uart| puts(uart, s));
}

/// Print to the console, as Plan 9's print, formatting straight to the UART,
/// so a long print, such as a TrapFrame's, isn't cut short by a buffer.
/// PRINTLOCK is held for the whole output, so two prints don't interleave,
/// and a Display that printed would deadlock.
pub fn print(args: fmt::Arguments) {
    with_printlock(|uart| {
        let _ = fmt::write(&mut Writer(uart), args);
    });
}

/// Run `f` on the UART with PRINTLOCK held, if there's a UART.  PRINTLOCK
/// is taken with IRQs unmasked, so a handler that printed while its CPU held
/// it would deadlock: handlers use iprint.
fn with_printlock(f: impl FnOnce(&'static dyn Uart)) {
    assert!(!irq::in_interrupt(), "print in interrupt context");
    let node = LockNode::new();
    let _print = PRINTLOCK.lock(&node);
    if let Some(uart) = uart() {
        f(uart);
    }
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

/// How many bytes of input the ring holds, and the longest line, as Plan 9's
/// kbd.istage and kbd.line.
const RING: usize = 1024;
const LINE: usize = 1024;

/// The console's input, as Plan 9's kbd: the bytes the UART's receive
/// interrupt has put, not yet read, in a ring, and the line a reader edits
/// from them.  The interrupt only puts and echoes; the editing, Plan 9's
/// cooked mode, is the reader's, in process context.
struct Input {
    /// The bytes put, not yet edited: ring[r], and the n - 1 after it,
    /// wrapping.
    ring: [u8; RING],
    r: usize,
    n: usize,
    /// How many of them end a line: newlines and ^Ds.
    ends: usize,
    /// The line being edited, line[..x].  Once it's sent, it's read from
    /// line[pos..x], and nothing more is edited until it's all read.
    line: [u8; LINE],
    x: usize,
    pos: usize,
    sent: bool,
}

impl Input {
    const fn new() -> Input {
        Input {
            ring: [0; RING],
            r: 0,
            n: 0,
            ends: 0,
            line: [0; LINE],
            x: 0,
            pos: 0,
            sent: false,
        }
    }

    /// Put `b` in the ring, and return whether it's kept: when the ring is
    /// full, it's dropped, as Plan 9's kbdcr2nl drops it.
    fn put(&mut self, b: u8) -> bool {
        if self.n == RING {
            return false;
        }
        self.ring[(self.r + self.n) % RING] = b;
        self.n += 1;
        if matches!(b, b'\n' | CTLD) {
            self.ends += 1;
        }
        true
    }

    /// Whether a reader has a line to read, or can edit one: a line is sent,
    /// the ring holds a line's end, or it's full.
    fn ready(&self) -> bool {
        self.sent || self.ends > 0 || self.n == RING
    }

    /// Edit the line with the ring's bytes, as Plan 9's consread does in
    /// cooked mode, until it's sent or the ring is empty: a backspace erases
    /// a byte, ^U the line, and a newline, which the line keeps, or a ^D,
    /// which it doesn't, sends it, as does a full line.
    fn edit(&mut self) {
        while !self.sent && self.n > 0 {
            let b = self.ring[self.r];
            self.r = (self.r + 1) % RING;
            self.n -= 1;
            match b {
                BACKSPACE => self.x = self.x.saturating_sub(1),
                CTLU => self.x = 0,
                b'\n' | CTLD => {
                    self.ends -= 1;
                    if b == b'\n' {
                        self.line[self.x] = b;
                        self.x += 1;
                    }
                    self.sent = true;
                }
                _ => {
                    self.line[self.x] = b;
                    self.x += 1;
                }
            }
            self.sent |= self.x == LINE;
        }
    }

    /// Copy what fits of the sent line into `buf`, and return its length.
    /// Once the line's all read, the next is edited.  A line sent by ^D
    /// alone is empty, so a read of it returns 0, the end of the file.
    fn take(&mut self, buf: &mut [u8]) -> usize {
        let m = buf.len().min(self.x - self.pos);
        buf[..m].copy_from_slice(&self.line[self.pos..self.pos + m]);
        self.pos += m;
        if self.pos == self.x {
            (self.x, self.pos, self.sent) = (0, 0, false);
        }
        m
    }
}

#[cfg(not(test))]
static INPUT: Lock<Input> = Lock::new("input", Input::new());

// Tests run in parallel threads, so each has its own.
#[cfg(test)]
std::thread_local! {
    static INPUT: &'static Lock<Input> = Box::leak(Box::new(Lock::new("input", Input::new())));
}

/// The console's input.  It's taken with ilock, since the receive interrupt
/// takes it, and never held across a sleep: a reader's sleep takes READER's
/// lock, then this one, to check its condition.
fn input() -> &'static Lock<Input> {
    core::cfg_select! {
        test => INPUT.with(|i| *i),
        _ => &INPUT,
    }
}

/// Where a reader waits for a line.
static READER: Rendez = Rendez::new();

/// Held by the process reading, as Plan 9's qlock(&kbd): one reads at a
/// time, as READER has one sleeper, and another waits its turn.
#[cfg(not(test))]
static KBD: QLock = QLock::new();

#[cfg(test)]
std::thread_local! {
    static KBD: &'static QLock = Box::leak(Box::new(QLock::new()));
}

fn kbd() -> &'static QLock {
    core::cfg_select! {
        test => KBD.with(|k| *k),
        _ => &KBD,
    }
}

/// Put the bytes the UART has received, as Plan 9's kbdcr2nl: a return
/// becomes a newline, and anything else is kept as it is, a delete included,
/// which only a backspace erases.  Each byte kept is echoed, and a reader is
/// woken when a line ends or the ring fills.  Input the ring has no room
/// for is dropped, without a report, as kbdcr2nl drops it.  It's the UART's
/// receive interrupt that calls it, so it doesn't allocate, and its work is
/// bounded by `bytes`.
pub fn put_input(bytes: &[u8]) {
    let mut wake = false;
    for &b in bytes {
        let b = if b == b'\r' { b'\n' } else { b };
        let node = LockNode::new();
        let mut input = input().ilock(&node);
        if !input.put(b) {
            continue;
        }
        wake |= input.ready();
        drop(input);
        echo(b);
    }
    if wake {
        READER.wakeup();
    }
}

/// Echo a byte of input, as Plan 9's echoserialoq: a ^U as "^U" and a
/// newline, and anything else as puts writes it.  It writes straight to the
/// UART, as iprint does, since it's called from an interrupt handler.
fn echo(b: u8) {
    let Some(uart) = uart() else {
        return;
    };
    match b {
        CTLU => puts(uart, b"^U\n"),
        b => puts(uart, &[b]),
    }
}

/// Read a line of input into `buf`, waiting until one is typed, and return
/// how much of it fits, as Plan 9's consread of cons: at most one line a
/// read, and what doesn't fit is the next read's.  0 is the end of the file,
/// typed as ^D at the start of a line.  One process reads at a time; another
/// waits its turn, as Plan 9's does on kbd's qlock.
pub fn read(buf: &mut [u8]) -> crate::Result<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    kbd().qlock();
    let n = loop {
        READER.sleep(|| {
            let node = LockNode::new();
            input().ilock(&node).ready()
        });
        let node = LockNode::new();
        let mut input = input().ilock(&node);
        input.edit();
        if input.sent {
            break input.take(buf);
        }
    };
    kbd().qunlock();
    Ok(n)
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

    /// Register the fake's current process, so a read can sleep: with its
    /// condition already holding, sleep returns without switching.
    fn reader() {
        static CURRENT: crate::proc::Current = crate::proc::Current::of::<crate::fake::Fake>();
        crate::proc::init_current(&CURRENT);
    }

    /// What a read of at most `n` bytes gives.
    fn read_n(n: usize) -> crate::Result<std::vec::Vec<u8>> {
        let mut buf = std::vec![0u8; n];
        let m = read(&mut buf)?;
        buf.truncate(m);
        Ok(buf)
    }

    #[test]
    fn a_line_is_echoed_and_read_whole() {
        reader();
        fake_cons();
        put_input(b"hello\r");
        assert_eq!(cons_output(), b"hello\r\n", "the echo, the return a newline");
        assert_eq!(read_n(100), Ok(b"hello\n".to_vec()));
    }

    #[test]
    fn erasing() {
        reader();
        fake_cons();
        // A backspace erases a byte, and ^U the line; a delete is kept, as
        // Plan 9's consread has no case for it.
        put_input(b"helx\x08p\n");
        assert_eq!(read_n(100), Ok(b"help\n".to_vec()), "backspace");
        put_input(b"helx\x7fp\n");
        assert_eq!(read_n(100), Ok(b"helx\x7fp\n".to_vec()), "delete");
        put_input(b"\x08\x08ok\n");
        assert_eq!(read_n(100), Ok(b"ok\n".to_vec()), "backspace on an empty line");
        cons_output();
        put_input(b"junk\x15ok\n");
        assert_eq!(cons_output(), b"junk^U\r\nok\r\n", "^U's echo");
        assert_eq!(read_n(100), Ok(b"ok\n".to_vec()), "^U");
    }

    #[test]
    fn ctl_d_sends_the_line_without_itself() {
        reader();
        fake_cons();
        put_input(b"abc\x04");
        assert_eq!(read_n(100), Ok(b"abc".to_vec()), "a line ended by ^D");
        put_input(b"\x04");
        assert_eq!(read_n(100), Ok(b"".to_vec()), "^D alone, the end of the file");
    }

    #[test]
    fn a_line_longer_than_the_read_is_read_in_parts() {
        reader();
        fake_cons();
        put_input(b"abcdef\nnext\n");
        assert_eq!(read_n(4), Ok(b"abcd".to_vec()));
        assert_eq!(read_n(100), Ok(b"ef\n".to_vec()), "the rest, and no more");
        assert_eq!(read_n(100), Ok(b"next\n".to_vec()), "a line typed ahead");
    }

    #[test]
    fn input_past_a_full_ring_is_dropped_silently() {
        reader();
        fake_cons();
        put_input(&[b'x'; RING + 6]);
        assert_eq!(cons_output(), [b'x'; RING], "only what's kept is echoed, and no report");
        // A full ring is a full line, sent without a newline.
        assert_eq!(read_n(RING), Ok(std::vec![b'x'; LINE]));
    }

    #[test]
    fn a_read_gives_up_the_reader_lock() {
        reader();
        fake_cons();
        put_input(b"a\r");
        assert_eq!(read_n(100), Ok(b"a\n".to_vec()));
        assert!(kbd().canqlock(), "the reader's lock is free");
        kbd().qunlock();
    }
}
