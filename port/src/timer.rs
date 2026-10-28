//! Timers, as Plan 9's portclock: one queue, sorted by deadline, with the
//! clock's timer armed for the earliest.  A timer's callback runs in
//! interrupt context, so it must be bounded, and mustn't allocate.

use crate::clock;
use crate::iprintln;
use crate::mcslock::{Lock, LockNode};
use core::cell::Cell;
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

/// The clock's frequency, as Plan 9's HZ.
pub const HZ: u64 = 100;

/// How many timers `interrupt` fires before reporting that it seems stuck, as
/// Plan 9's Maxtimerloops.
const STUCK_LOOPS: usize = 20_000;

/// The shortest period, as Plan 9's tadd asserts: a shorter one could be due
/// again before `interrupt` returns.
const MIN_PERIOD: Duration = Duration::from_micros(100);

pub enum Mode {
    /// Fire once, this long after `add`.
    Relative(Duration),
    /// Fire every period, from one period after `add`.
    Periodic(Duration),
}

pub struct Timer {
    mode: Mode,
    f: fn(),
    /// The deadline, in counts, the next timer in the queue, and whether
    /// it's queued.  These are touched only with TIMERS held.
    when: Cell<u64>,
    next: Cell<*const Timer>,
    queued: Cell<bool>,
}

// SAFETY: a Timer's Cells are only touched with TIMERS held.
unsafe impl Sync for Timer {}

impl Timer {
    pub const fn new(mode: Mode, f: fn()) -> Timer {
        Timer {
            mode,
            f,
            when: Cell::new(0),
            next: Cell::new(ptr::null()),
            queued: Cell::new(false),
        }
    }
}

/// The queued timers, linked through Timer::next in deadline order.
struct Queue {
    head: Cell<*const Timer>,
}

#[cfg(not(test))]
static TIMERS: Lock<Queue> = Lock::new("timers", Queue { head: Cell::new(ptr::null()) });

// Tests run in parallel threads, so each has its own queue.
#[cfg(test)]
std::thread_local! {
    static TIMERS: &'static Lock<Queue> =
        Box::leak(Box::new(Lock::new("timers", Queue { head: Cell::new(ptr::null()) })));
}

fn timers() -> &'static Lock<Queue> {
    core::cfg_select! {
        test => TIMERS.with(|t| *t),
        _ => &TIMERS,
    }
}

/// A queued timer.
fn timer(t: *const Timer) -> Option<&'static Timer> {
    // SAFETY: only &'static Timers are queued, so a link is null or valid.
    unsafe { t.as_ref() }
}

impl Queue {
    fn head(&self) -> Option<&'static Timer> {
        timer(self.head.get())
    }

    /// Queue `t` in deadline order, after any timer with the same deadline.
    /// The head is just the first link, so it needs no case of its own.
    fn insert(&self, t: &'static Timer) {
        let mut link = &self.head;
        while let Some(n) = timer(link.get()).filter(|n| n.when.get() <= t.when.get()) {
            link = &n.next;
        }
        t.next.set(link.get());
        link.set(t);
        t.queued.set(true);
    }

    /// Unqueue `t`, if it's queued.
    fn remove(&self, t: &'static Timer) {
        if !t.queued.get() {
            return;
        }
        let mut link = &self.head;
        while let Some(n) = timer(link.get()).filter(|&n| !ptr::eq(n, t)) {
            link = &n.next;
        }
        link.set(t.next.get());
        t.queued.set(false);
    }

    /// Arm the clock's timer for the earliest deadline, or disarm it.
    fn arm(&self) {
        clock::set_timer(self.head().map(|t| t.when.get()));
    }
}

fn now_and_hz() -> (u64, u64) {
    let now = clock::now().expect("timer: no clock");
    (now, clock::frequency().expect("timer: no clock"))
}

/// Queue `t` to fire after its duration, moving it if it's already queued,
/// as Plan 9's timeradd.
pub fn add(t: &'static Timer) {
    let d = match t.mode {
        Mode::Relative(d) => {
            assert!(!d.is_zero(), "add: zero duration");
            d
        }
        Mode::Periodic(p) => {
            assert!(p >= MIN_PERIOD, "add: period under {MIN_PERIOD:?}");
            p
        }
    };
    let node = LockNode::new();
    let q = timers().ilock(&node);
    q.remove(t);
    let (now, hz) = now_and_hz();
    t.when.set(now + clock::to_counts(d, hz));
    q.insert(t);
    q.arm();
}

/// Unqueue `t`, so it doesn't fire, as Plan 9's timerdel.  A periodic timer
/// can't stop itself this way from its own callback: it's unqueued while the
/// callback runs, and `interrupt` re-queues it after.
pub fn remove(t: &'static Timer) {
    let node = LockNode::new();
    let q = timers().ilock(&node);
    q.remove(t);
    q.arm();
}

/// Fire every timer whose deadline has passed, then arm the clock's timer for
/// the next, as Plan 9's timerintr.  The architecture calls it from the
/// clock's timer interrupt.  A periodic timer's next deadline is its last
/// plus its period, so it doesn't drift.
pub fn interrupt() {
    let (now, hz) = now_and_hz();
    let node = LockNode::new();
    let mut q = timers().ilock(&node);
    let mut fired = 0;
    while let Some(t) = q.head().filter(|t| t.when.get() <= now) {
        q.remove(t);
        drop(q);
        (t.f)();
        q = timers().ilock(&node);
        if let Mode::Periodic(period) = t.mode
            && !t.queued.get()
        {
            t.when.set(t.when.get() + clock::to_counts(period, hz));
            q.insert(t);
        }
        fired += 1;
        if fired % STUCK_LOOPS == 0 {
            iprintln!("interrupt: probably stuck in its loop");
        }
    }
    q.arm();
}

/// Clock ticks since `start`, as Plan 9's m->ticks.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// The HZ clock, as Plan 9's hzclock.
fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

static HZCLOCK: Timer = Timer::new(Mode::Periodic(Duration::from_nanos(1_000_000_000 / HZ)), tick);

/// Start the HZ clock, as Plan 9's timersinit.
pub fn start() {
    add(&HZCLOCK);
}

/// Clock ticks since `start`.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(0) };
        static ARMED: Cell<Option<u64>> = const { Cell::new(None) };
        static FIRED: RefCell<Vec<char>> = const { RefCell::new(Vec::new()) };
    }

    /// A 1 kHz clock, so a tick is a millisecond, whose time the test sets.
    static FAKE: clock::Ops =
        clock::Ops { now: || NOW.get(), frequency: || 1000, set_timer: |when| ARMED.set(when) };

    fn set_up() {
        clock::init(&FAKE);
    }

    fn at(ms: u64) {
        NOW.set(ms);
        interrupt();
    }

    fn fired() -> Vec<char> {
        FIRED.with(|f| f.take())
    }

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn timers_fire_in_deadline_order() {
        static A: Timer =
            Timer::new(Mode::Relative(ms(10)), || FIRED.with(|f| f.borrow_mut().push('a')));
        static B: Timer =
            Timer::new(Mode::Relative(ms(30)), || FIRED.with(|f| f.borrow_mut().push('b')));
        static C: Timer =
            Timer::new(Mode::Relative(ms(20)), || FIRED.with(|f| f.borrow_mut().push('c')));
        set_up();
        add(&B);
        add(&A);
        add(&C);
        assert_eq!(ARMED.get(), Some(10));
        at(25);
        assert_eq!(fired(), ['a', 'c']);
        assert_eq!(ARMED.get(), Some(30));
        at(30);
        assert_eq!(fired(), ['b']);
        assert_eq!(ARMED.get(), None);
    }

    #[test]
    fn periodic_deadlines_dont_drift() {
        static P: Timer =
            Timer::new(Mode::Periodic(ms(10)), || FIRED.with(|f| f.borrow_mut().push('p')));
        set_up();
        add(&P);
        // Late by 3 ms, it still next fires at 20, not 23.
        at(13);
        assert_eq!(fired(), ['p']);
        assert_eq!(ARMED.get(), Some(20));
        // Late by more than a period, it catches up.
        at(45);
        assert_eq!(fired(), ['p', 'p', 'p']);
        assert_eq!(ARMED.get(), Some(50));
        remove(&P);
    }

    #[test]
    fn remove_stops_a_timer() {
        static A: Timer =
            Timer::new(Mode::Relative(ms(10)), || FIRED.with(|f| f.borrow_mut().push('a')));
        set_up();
        add(&A);
        remove(&A);
        assert_eq!(ARMED.get(), None);
        at(20);
        assert_eq!(fired(), []);
    }

    #[test]
    fn armed_for_the_head() {
        static A: Timer = Timer::new(Mode::Relative(ms(20)), || {});
        static B: Timer = Timer::new(Mode::Relative(ms(10)), || {});
        set_up();
        add(&A);
        assert_eq!(ARMED.get(), Some(20));
        add(&B);
        assert_eq!(ARMED.get(), Some(10));
        remove(&B);
        assert_eq!(ARMED.get(), Some(20));
        // Adding a queued timer moves it.
        NOW.set(5);
        add(&A);
        assert_eq!(ARMED.get(), Some(25));
        remove(&A);
        assert_eq!(ARMED.get(), None);
    }

    #[test]
    #[should_panic(expected = "period under")]
    fn short_period_panics() {
        static S: Timer = Timer::new(Mode::Periodic(Duration::from_micros(99)), || {});
        set_up();
        add(&S);
    }

    #[test]
    #[should_panic(expected = "zero duration")]
    fn zero_duration_panics() {
        static Z: Timer = Timer::new(Mode::Relative(Duration::ZERO), || {});
        set_up();
        add(&Z);
    }
}
