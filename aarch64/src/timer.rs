//! Minimal timer subsystem using the ARMv8 architectural timer.
//!
//! Timers are caller-owned: the subsystem stores only `&'static`
//! references in a small fixed table, never allocates, and the
//! interrupt handler frees nothing.  Timers therefore live in statics,
//! with interior mutability making them shareable with the handler.

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;

use port::irq::IrqGuard;
use port::mcslock::{Lock, LockNode};

use crate::reg::cnt_el0::{CntFrqEl0, CntPctEl0, CntpCtlEl0, CntpCvalEl0};

/// Fired in interrupt context.  Return true to keep a periodic timer
/// running; the return value is ignored for one-shot timers.
pub trait TimerCallback: Send + Sync {
    fn fire(&self) -> bool;
}

/// A caller-owned timer.  `start` registers it with the subsystem,
/// which only ever borrows it.  Not designed for concurrent restarts
/// of the same timer.
pub struct Timer {
    duration: Duration,
    repeat: bool,
    deadline_ticks: AtomicU64,
    period_ticks: AtomicU64,
    active: AtomicBool,
    callback: &'static dyn TimerCallback,
}

impl Timer {
    /// A one-shot timer firing once, `relative` after `start`.
    pub const fn new(relative: Duration, callback: &'static dyn TimerCallback) -> Self {
        Self {
            duration: relative,
            repeat: false,
            deadline_ticks: AtomicU64::new(0),
            period_ticks: AtomicU64::new(0),
            active: AtomicBool::new(false),
            callback,
        }
    }

    /// A timer firing every `period` until its callback returns false
    /// or it is cancelled.
    pub const fn periodic(period: Duration, callback: &'static dyn TimerCallback) -> Self {
        let mut timer = Self::new(period, callback);
        timer.repeat = true;
        timer
    }

    /// Register and arm the timer.  Panics if the timer table is full.
    pub fn start(&'static self) {
        let ticks = duration_to_ticks(self.duration);
        self.period_ticks.store(if self.repeat { ticks } else { 0 }, Ordering::Relaxed);
        self.deadline_ticks.store(now() + ticks, Ordering::Relaxed);
        self.active.store(true, Ordering::Release);
        let _irq = IrqGuard::new();
        register(self);
        arm_hardware();
    }

    /// Deactivate the timer.  Lazy: an already-armed hardware deadline
    /// may still cause one spurious wakeup.  Idempotent, and safe to
    /// call from any callback, including the timer's own.
    pub fn cancel(&self) {
        self.active.store(false, Ordering::Release);
    }
}

fn timer_enable() {
    if !cfg!(test) {
        CntpCtlEl0::write(CntpCtlEl0::read().with_enable(true));
    }
}

fn timer_disable() {
    if !cfg!(test) {
        CntpCtlEl0::write(CntpCtlEl0::read().with_enable(false));
    }
}

const MAX_TIMERS: usize = 8;

static TIMERS: Lock<[Option<&'static Timer>; MAX_TIMERS]> = Lock::new("timers", [None; MAX_TIMERS]);

/// Run `f` with the timer table locked and IRQs masked.  The lock is
/// shared with the interrupt handler, so it must never be held with
/// IRQs enabled: a timer interrupt arriving mid-hold would spin on it
/// forever.  (In the handler itself the masking is a harmless no-op.)
fn with_timers<R>(f: impl FnOnce(&mut [Option<&'static Timer>; MAX_TIMERS]) -> R) -> R {
    let _irq = IrqGuard::new();
    let node = LockNode::new();
    let mut guard = TIMERS.lock(&node);
    f(&mut guard)
}

fn register(timer: &'static Timer) {
    with_timers(|timers| {
        // Already registered (a restart)?
        if timers.iter().flatten().any(|t| ptr::eq(*t, timer)) {
            return;
        }
        // Take a free slot, or one whose timer is no longer active.
        for slot in timers.iter_mut() {
            if slot.is_none_or(|t| !t.active.load(Ordering::Acquire)) {
                *slot = Some(timer);
                return;
            }
        }
        panic!("timer table full");
    })
}

/// Arm the hardware for the earliest active deadline, or disarm if
/// there is none.  Call with IRQs masked.
fn arm_hardware() {
    let timers = with_timers(|timers| *timers);
    let next = timers
        .into_iter()
        .flatten()
        .filter(|t| t.active.load(Ordering::Acquire))
        .map(|t| t.deadline_ticks.load(Ordering::Relaxed))
        .min();
    match next {
        Some(deadline) => {
            CntpCvalEl0::write(deadline);
            timer_enable();
        }
        None => timer_disable(),
    }
}

static TIMER_FREQ: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    let freq = CntFrqEl0::read().freq();
    if freq == 0 {
        panic!("timer: CNTFRQ_EL0=0: counter frequency not programmed by firmware");
    }
    TIMER_FREQ.store(freq, Ordering::Relaxed);
}

fn now() -> u64 {
    CntPctEl0::read().value()
}

/// Convert a duration to hardware ticks.  A zero tick count would arm
/// a periodic timer with an unchanging, always-past deadline — an
/// interrupt storm — so a timer started before `init` is a bug.
fn duration_to_ticks(dur: Duration) -> u64 {
    let freq = TIMER_FREQ.load(Ordering::Relaxed);
    ((dur.as_nanos() * freq as u128) / 1_000_000_000) as u64
}

/// Hardware interrupt handler — called from the trap handler.
///
/// Fires all due timers (outside the table lock, so a callback may
/// start or cancel timers) and arms the next deadline, which is also
/// what deasserts the level-triggered timer interrupt: the new CVAL is
/// in the future, or the timer is disabled.
pub fn interrupt_handler() {
    if cfg!(test) {
        return;
    }

    // 1. Copy out the table so callbacks run outside the lock.
    let timers = with_timers(|timers| *timers);

    // 2. Fire due timers.
    let now = now();
    for timer in timers.into_iter().flatten() {
        if !timer.active.load(Ordering::Acquire) {
            continue;
        }
        let deadline = timer.deadline_ticks.load(Ordering::Relaxed);
        if deadline > now {
            continue;
        }
        let period = timer.period_ticks.load(Ordering::Relaxed);
        if period == 0 {
            // Deactivate before firing so the callback may restart it.
            timer.active.store(false, Ordering::Release);
            timer.callback.fire();
        } else if timer.callback.fire() {
            // Advance the deadline; if the callback cancelled its own
            // timer the cleared active flag still stops it.
            timer.deadline_ticks.store(deadline + period, Ordering::Relaxed);
        } else {
            timer.active.store(false, Ordering::Release);
        }
    }

    // 3. Arm next timer or disarm, deasserting the interrupt.
    arm_hardware();
}
