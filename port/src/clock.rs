//! The monotonic clock, as Plan 9's fastticks: a counter that counts up at a
//! fixed frequency, for code that isn't generic over the architecture.  Each
//! architecture registers its functions with init.  Until it does, there's
//! no clock.

use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use core::time::Duration;

/// An architecture's clock functions.
pub struct Ops {
    /// The counter, which never goes backwards.
    pub fastticks: fn() -> u64,
    /// The counter's frequency, in Hz.
    pub fasthz: fn() -> u64,
    /// Arm the timer to interrupt once the counter reaches a deadline, or
    /// disarm it, as Plan 9's timerset.  A deadline in the future quietens
    /// the interrupt before it returns.
    pub timerset: fn(Option<u64>),
}

#[cfg(not(test))]
static OPS: AtomicPtr<Ops> = AtomicPtr::new(ptr::null_mut());

// Tests run in parallel threads, so each registers its own functions.
#[cfg(test)]
std::thread_local! {
    static OPS: AtomicPtr<Ops> = const { AtomicPtr::new(ptr::null_mut()) };
}

/// Register the architecture's clock functions.
pub fn init(ops: &'static Ops) {
    assert!((ops.fasthz)() != 0, "clock: the counter's frequency is 0: firmware didn't set it");
    let ops = ptr::from_ref(ops).cast_mut();
    core::cfg_select! {
        test => OPS.with(|o| o.store(ops, Ordering::Release)),
        _ => OPS.store(ops, Ordering::Release),
    }
}

fn ops() -> Option<&'static Ops> {
    let ops = core::cfg_select! {
        test => OPS.with(|o| o.load(Ordering::Acquire)),
        _ => OPS.load(Ordering::Acquire),
    };
    // SAFETY: OPS is null, or was set from a &'static Ops by init.
    unsafe { ops.as_ref() }
}

/// The counter, or None if there's no clock.
pub fn fastticks() -> Option<u64> {
    ops().map(|o| (o.fastticks)())
}

/// The counter's frequency, in Hz, or None if there's no clock.
pub fn fasthz() -> Option<u64> {
    ops().map(|o| (o.fasthz)())
}

/// Arm the timer for a deadline in fastticks, or disarm it.  Without a clock
/// there's no timer, so it does nothing.
pub fn timerset(when: Option<u64>) {
    if let Some(o) = ops() {
        (o.timerset)(when);
    }
}

const NS_PER_SEC: u128 = 1_000_000_000;

/// `d` as a count of fastticks at `hz`, rounded up, so a deadline is never
/// early.  It saturates at u64::MAX.
pub fn ns2fastticks(d: Duration, hz: u64) -> u64 {
    let t = d.as_nanos().saturating_mul(hz.into()).div_ceil(NS_PER_SEC);
    u64::try_from(t).unwrap_or(u64::MAX)
}

/// `t` fastticks at `hz`, which mustn't be 0, as a duration, rounded down.
pub fn fastticks2ns(t: u64, hz: u64) -> Duration {
    let nanos = u128::from(t % hz) * NS_PER_SEC / u128::from(hz);
    Duration::new(t / hz, nanos as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PI4: u64 = 54_000_000;
    const QEMU: u64 = 62_500_000;
    const GHZ: u64 = 1_000_000_000;

    #[test]
    fn whole_seconds_are_exact() {
        for hz in [PI4, QEMU, GHZ] {
            assert_eq!(ns2fastticks(Duration::from_secs(3), hz), 3 * hz);
            assert_eq!(fastticks2ns(3 * hz, hz), Duration::from_secs(3));
        }
    }

    #[test]
    fn durations_round_up_and_counts_round_down() {
        // A tick at 54 MHz is 18.5 ns.
        assert_eq!(ns2fastticks(Duration::from_nanos(1), PI4), 1);
        assert_eq!(ns2fastticks(Duration::from_nanos(19), PI4), 2);
        assert_eq!(fastticks2ns(1, PI4), Duration::from_nanos(18));
        assert_eq!(fastticks2ns(2, PI4), Duration::from_nanos(37));
        // 62.5 MHz is 16 ns a tick, exactly.
        assert_eq!(ns2fastticks(Duration::from_nanos(16), QEMU), 1);
        assert_eq!(ns2fastticks(Duration::from_nanos(17), QEMU), 2);
        assert_eq!(ns2fastticks(Duration::ZERO, QEMU), 0);
    }

    #[test]
    fn round_trips_are_never_early() {
        for hz in [PI4, QEMU, GHZ] {
            for ns in [1, 17, 999, 16_666_667, 100_000_000, 123_456_789_012] {
                let d = Duration::from_nanos(ns);
                let back = fastticks2ns(ns2fastticks(d, hz), hz);
                let tick = Duration::from_nanos(GHZ.div_ceil(hz));
                assert!(back >= d && back - d < tick, "{ns} ns at {hz} Hz came back as {back:?}");
            }
        }
    }

    #[test]
    fn extremes_neither_overflow_nor_panic() {
        assert_eq!(fastticks2ns(u64::MAX, PI4).as_secs(), u64::MAX / PI4);
        assert_eq!(fastticks2ns(u64::MAX, GHZ).as_secs(), u64::MAX / GHZ);
        assert_eq!(ns2fastticks(Duration::MAX, PI4), u64::MAX);
        assert_eq!(ns2fastticks(Duration::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn nothing_without_ops() {
        assert_eq!(fastticks(), None);
        assert_eq!(fasthz(), None);
    }

    #[test]
    fn registered_ops_are_used() {
        static OPS: Ops = Ops { fastticks: || 42, fasthz: || PI4, timerset: |_| {} };
        init(&OPS);
        assert_eq!(fastticks(), Some(42));
        assert_eq!(fasthz(), Some(PI4));
    }

    #[test]
    #[should_panic(expected = "frequency is 0")]
    fn zero_frequency_panics() {
        static OPS: Ops = Ops { fastticks: || 0, fasthz: || 0, timerset: |_| {} };
        init(&OPS);
    }
}
