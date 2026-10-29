use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicI64, Ordering};
use core::time::Duration;
use core::{fmt, hint};

use crate::chan::{Chan, OREAD, Qid, QTDIR, QTFILE};
use crate::clock;
use crate::dev::{self, Dev};
use crate::error::{EBADTIMECTL, ENONEXIST, ENOTDIR, ENOTSUP, EPERM};
use crate::irq;
use crate::mcslock::{Lock, LockGuard, LockNode};
use crate::proc;

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

/// The console device, as Plan 9's devcons: the only device, reached as #c,
/// with the root directory and the files in FILES.
pub struct Cons;

/// The console device, the only member of the device table.
pub static CONS: Cons = Cons;

/// A file of the console device, as an entry of Plan 9's Dirtab.
struct File {
    name: &'static str,
    path: u64,
    mode: u32,
    /// The file's length, as consdir gives it: what a read at offset 0
    /// returns, or 0 for cons.
    length: u64,
}

/// The root directory's qid path, and its mode, as `.` in Plan 9's consdir.
const ROOT: u64 = 0;
const ROOT_MODE: u32 = 0o555;

const CONS_PATH: u64 = 1;
const TIME_PATH: u64 = 2;
const PID_PATH: u64 = 3;

/// The width of a formatted number, as Plan 9's NUMSIZE and VLNUMSIZE: the
/// digits right-aligned, then a space.
const NUMSIZE: usize = 12;
const VLNUMSIZE: usize = 22;

/// The console device's files, as Plan 9's consdir.  cons is write-only
/// until the console-input task, which makes it Plan 9's 0660.
const FILES: &[File] = &[
    File { name: "cons", path: CONS_PATH, mode: 0o220, length: 0 },
    File { name: "time", path: TIME_PATH, mode: 0o664, length: (NUMSIZE + 3 * VLNUMSIZE) as u64 },
    File { name: "pid", path: PID_PATH, mode: 0o444, length: NUMSIZE as u64 },
];

fn file(path: u64) -> Option<&'static File> {
    FILES.iter().find(|f| f.path == path)
}

/// What a write of seconds to time adds to the clock's time, in nanoseconds,
/// as Plan 9's todset: there's no real-time clock, so until it's set, the
/// time counts from when the clock's counter started.
#[cfg(not(test))]
static TIME_OFFSET: AtomicI64 = AtomicI64::new(0);

// Tests run in parallel threads, so each has its own.
#[cfg(test)]
std::thread_local! {
    static TIME_OFFSET: AtomicI64 = const { AtomicI64::new(0) };
}

fn time_offset() -> i64 {
    core::cfg_select! {
        test => TIME_OFFSET.with(|o| o.load(Ordering::Relaxed)),
        _ => TIME_OFFSET.load(Ordering::Relaxed),
    }
}

fn set_time_offset(offset: i64) {
    core::cfg_select! {
        test => TIME_OFFSET.with(|o| o.store(offset, Ordering::Relaxed)),
        _ => TIME_OFFSET.store(offset, Ordering::Relaxed),
    }
}

const NS_PER_SEC: i128 = 1_000_000_000;

/// The clock's time in nanoseconds, its count, and its frequency, or None
/// without a clock.  The count is from when the counter started: power-on,
/// on a Pi 4, so before the firmware ran, not when the kernel booted.
fn clock_time() -> Option<(i128, u64, u64)> {
    let (ticks, hz) = (clock::now()?, clock::frequency()?);
    Some((clock::to_duration(ticks, hz).as_nanos() as i128, ticks, hz))
}

/// The time file's text, as Plan 9's readtime: the seconds, the
/// nanoseconds, the clock's count, and its frequency, each right-aligned in
/// its width and followed by a space.
fn time_text() -> crate::Result<String> {
    let (nsec, ticks, hz) = clock_time().ok_or(ENOTSUP)?;
    let nsec = nsec + i128::from(time_offset());
    Ok(format!(
        "{:>w$} {:>vw$} {:>vw$} {:>vw$} ",
        nsec / NS_PER_SEC,
        nsec,
        ticks,
        hz,
        w = NUMSIZE - 1,
        vw = VLNUMSIZE - 1,
    ))
}

/// Set the time to the seconds written, as Plan 9's writetime: fewer than 13
/// bytes, holding a positive decimal number, with anything after its digits,
/// such as a newline, ignored.  Plan 9's strtol also takes hex and octal.
fn set_time(buf: &[u8]) -> crate::Result<()> {
    if buf.len() >= 13 {
        return Err(EBADTIMECTL);
    }
    let text = core::str::from_utf8(buf).map_err(|_| EBADTIMECTL)?.trim_start();
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let secs: i128 = text[..digits].parse().map_err(|_| EBADTIMECTL)?;
    if secs <= 0 {
        return Err(EBADTIMECTL);
    }
    let (nsec, _, _) = clock_time().ok_or(ENOTSUP)?;
    // Twelve digits of seconds can be more nanoseconds than an i64 holds,
    // where Plan 9's 32-bit long can't.
    let offset = i64::try_from(secs * NS_PER_SEC - nsec).map_err(|_| EBADTIMECTL)?;
    set_time_offset(offset);
    Ok(())
}

/// Copy the part of `s` from `off` that fits into `buf`, and return its
/// length, as Plan 9's readstr: nothing at or past its end.
fn readstr(off: u64, buf: &mut [u8], s: &[u8]) -> u64 {
    let rest = usize::try_from(off).ok().and_then(|o| s.get(o..)).unwrap_or_default();
    let n = rest.len().min(buf.len());
    buf[..n].copy_from_slice(&rest[..n]);
    n as u64
}

impl Dev for Cons {
    fn dc(&self) -> char {
        'c'
    }

    fn attach(&self) -> crate::Result<Arc<Chan>> {
        Ok(Chan::new(&CONS, Qid { path: ROOT, kind: QTDIR }))
    }

    fn walk(&self, c: &Arc<Chan>, elems: &[&str]) -> crate::Result<Arc<Chan>> {
        let mut c = c.clone();
        for elem in elems {
            if c.qid().kind != QTDIR {
                return Err(ENOTDIR);
            }
            let f = FILES.iter().find(|f| f.name == *elem).ok_or(ENONEXIST)?;
            c = Chan::new(&CONS, Qid { path: f.path, kind: QTFILE });
        }
        Ok(c)
    }

    fn open(&self, c: &Arc<Chan>, omode: u32) -> crate::Result<Arc<Chan>> {
        let (mode, length) = match file(c.qid().path) {
            Some(f) => (f.mode, f.length),
            None => (ROOT_MODE, 0),
        };
        // The permission check, as Plan 9's devpermcheck, comes before the
        // directory check, as devopen's order.
        if !dev::perm_ok(mode, omode) {
            return Err(EPERM);
        }
        // A directory opens OREAD only, as Plan 9's devopen, which errors
        // Eperm where it is not.
        if c.qid().kind == QTDIR && omode != OREAD {
            return Err(EPERM);
        }
        Ok(Chan::opened(&CONS, *c.qid(), omode, length))
    }

    fn read(&self, c: &Arc<Chan>, buf: &mut [u8], offset: u64) -> crate::Result<u64> {
        match c.qid().path {
            TIME_PATH => Ok(readstr(offset, buf, time_text()?.as_bytes())),
            PID_PATH => {
                let pid = proc::current_pid().ok_or(ENOTSUP)?;
                Ok(readstr(offset, buf, format!("{pid:>w$} ", w = NUMSIZE - 1).as_bytes()))
            }
            _ => Err(ENOTSUP),
        }
    }

    fn write(&self, c: &Arc<Chan>, buf: &[u8], _offset: u64) -> crate::Result<u64> {
        match c.qid().path {
            CONS_PATH => putstrn(buf),
            TIME_PATH => set_time(buf)?,
            _ => return Err(ENOTSUP),
        }
        Ok(buf.len() as u64)
    }

    fn close(&self, _c: &Chan) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chan::{ORDWR, OWRITE};
    use crate::dev::open_name;
    use crate::fake::{cons_masked, cons_output, fake_cons};
    use std::cell::Cell;

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

    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(0) };
    }

    /// A 1 kHz clock, so a count is a millisecond, whose count the test sets.
    static CLOCK: clock::Ops = clock::Ops { now: || NOW.get(), frequency: || 1000, set_timer: |_| {} };

    /// What a read of `c` at `offset` gives, into a buffer of `n` bytes.
    fn read(c: &Arc<Chan>, offset: u64, n: usize) -> crate::Result<std::vec::Vec<u8>> {
        let mut buf = std::vec![0u8; n];
        let m = c.dev().read(c, &mut buf, offset)?;
        buf.truncate(m as usize);
        Ok(buf)
    }

    #[test]
    fn opens_time_and_pid() {
        let time = open_name("#c/time", ORDWR).expect("time, read and write");
        assert_eq!(time.length(), 78, "NUMSIZE plus three VLNUMSIZEs");
        let pid = open_name("#c/pid", OREAD).expect("pid");
        assert_eq!(pid.length(), 12, "NUMSIZE");
        assert_eq!(open_name("#c/pid", OWRITE).err(), Some(EPERM), "pid is read-only");
    }

    #[test]
    fn time_reads_and_is_set_by_seconds() {
        clock::init(&CLOCK);
        let c = open_name("#c/time", ORDWR).expect("time");

        // Seconds, nanoseconds, the count and its frequency, each
        // right-aligned in its width, then a space.
        NOW.set(2500);
        let text = read(&c, 0, 100).expect("a read");
        assert_eq!(text.len(), 78);
        for end in [11, 33, 55, 77] {
            assert_eq!(text[end], b' ', "a field ends at {end}");
        }
        let fields: std::vec::Vec<&str> =
            core::str::from_utf8(&text).unwrap().split_whitespace().collect();
        assert_eq!(fields, ["2", "2500000000", "2500", "1000"], "since the counter started");

        // A read from an offset gets the rest, and past the end, nothing.
        assert_eq!(read(&c, 70, 100).expect("the tail"), &text[70..]);
        assert_eq!(read(&c, 78, 100).expect("the end"), b"");

        // A write of seconds sets the time; the count is the clock's.
        assert_eq!(c.dev().write(&c, b"1000000\n", 0), Ok(8));
        NOW.set(3500);
        let text = read(&c, 0, 100).expect("a read");
        let fields: std::vec::Vec<&str> =
            core::str::from_utf8(&text).unwrap().split_whitespace().collect();
        assert_eq!(fields, ["1000001", "1000001000000000", "3500", "1000"], "after the set");

        // What Plan 9's writetime refuses, and what i64 nanoseconds can't
        // hold.
        for bad in [&b"0"[..], b"-5", b"abc", b"", b"1234567890123", b"999999999999"] {
            assert_eq!(c.dev().write(&c, bad, 0), Err(EBADTIMECTL), "{bad:?}");
        }
    }

    #[test]
    fn pid_reads_the_current_process() {
        let c = open_name("#c/pid", OREAD).expect("pid");
        assert_eq!(read(&c, 0, 100), Err(ENOTSUP), "without a current process");

        static CURRENT: proc::Current = proc::Current { pid: || Some(42) };
        proc::init_current(&CURRENT);
        assert_eq!(read(&c, 0, 100).expect("a read"), b"         42 ");
        assert_eq!(read(&c, 10, 100).expect("the tail"), b"2 ");
    }
}
