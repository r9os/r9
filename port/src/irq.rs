//! Masking IRQs, and tracking interrupt context, for code that isn't generic
//! over the architecture: the console, the allocators and the locks.  Each
//! architecture registers its functions with init.  Until it does, masking
//! does nothing and nothing is in interrupt context.

use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// The IRQ state that masking saved, for restoring.  What it holds is up to
/// the architecture: on aarch64 it's DAIF.
#[derive(Clone, Copy)]
pub struct Saved(pub u64);

/// An architecture's IRQ functions.
pub struct Ops {
    /// Mask IRQs on this CPU, and return the state to restore.
    pub mask: fn() -> Saved,
    /// Restore the state `mask` returned.
    pub restore: fn(Saved),
    /// This CPU's interrupt nesting depth.
    pub depth: fn() -> &'static AtomicUsize,
}

#[cfg(not(test))]
static OPS: AtomicPtr<Ops> = AtomicPtr::new(ptr::null_mut());

// Tests run in parallel threads, so each registers its own functions.
#[cfg(test)]
std::thread_local! {
    static OPS: AtomicPtr<Ops> = const { AtomicPtr::new(ptr::null_mut()) };
}

/// Register the architecture's IRQ functions.
pub fn init(ops: &'static Ops) {
    let ops = ptr::from_ref(ops).cast_mut();
    #[cfg(not(test))]
    OPS.store(ops, Ordering::Release);
    #[cfg(test)]
    OPS.with(|o| o.store(ops, Ordering::Release));
}

fn ops() -> Option<&'static Ops> {
    #[cfg(not(test))]
    let ops = OPS.load(Ordering::Acquire);
    #[cfg(test)]
    let ops = OPS.with(|o| o.load(Ordering::Acquire));
    // SAFETY: OPS is null, or was set from a &'static Ops by init.
    unsafe { ops.as_ref() }
}

/// Mask IRQs, returning the state to restore, as Plan 9's splhi does.
pub(crate) fn mask() -> Option<Saved> {
    ops().map(|o| (o.mask)())
}

/// Restore the state `mask` returned, as Plan 9's splx does.
pub(crate) fn restore(saved: Option<Saved>) {
    if let (Some(o), Some(s)) = (ops(), saved) {
        (o.restore)(s);
    }
}

/// Run `f` with IRQs masked, then restore their previous state, so calls
/// nest.  `f` mustn't sleep or switch processes.
pub fn masked<R>(f: impl FnOnce() -> R) -> R {
    let saved = mask();
    let r = f();
    restore(saved);
    r
}

/// Note that this CPU has started handling an interrupt.
pub fn enter_interrupt() {
    if let Some(o) = ops() {
        (o.depth)().fetch_add(1, Ordering::Relaxed);
    }
}

/// Note that this CPU has finished handling an interrupt.
pub fn exit_interrupt() {
    if let Some(o) = ops() {
        let depth = (o.depth)().fetch_sub(1, Ordering::Relaxed);
        assert!(depth > 0, "exit_interrupt without enter_interrupt");
    }
}

/// Whether this CPU is handling an interrupt.
pub fn in_interrupt() -> bool {
    ops().is_some_and(|o| (o.depth)().load(Ordering::Relaxed) > 0)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mcslock::{Lock, LockNode};
    use std::cell::Cell;

    thread_local! {
        static MASKED: Cell<bool> = const { Cell::new(false) };
        static DEPTH: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
    }

    /// Fake IRQ functions, whose state is per thread.
    pub(crate) static FAKE: Ops = Ops {
        mask: || Saved(MASKED.replace(true) as u64),
        restore: |s| MASKED.set(s.0 != 0),
        depth: || DEPTH.with(|d| *d),
    };

    fn is_masked() -> bool {
        MASKED.get()
    }

    #[test]
    fn masked_nests_and_restores() {
        init(&FAKE);
        assert!(!is_masked());
        masked(|| {
            assert!(is_masked());
            masked(|| assert!(is_masked()));
            assert!(is_masked());
        });
        assert!(!is_masked());
    }

    #[test]
    fn ilock_masks_while_held() {
        init(&FAKE);
        let lock = Lock::new("test", 0);
        let node = LockNode::new();
        {
            let mut g = lock.ilock(&node);
            assert!(is_masked());
            *g += 1;
        }
        assert!(!is_masked());

        // Held inside a masked section, it leaves IRQs masked.
        masked(|| {
            drop(lock.ilock(&node));
            assert!(is_masked());
        });
        assert!(!is_masked());

        // The ordinary lock doesn't mask.
        let _g = lock.lock(&node);
        assert!(!is_masked());
    }

    #[test]
    fn interrupt_depth() {
        init(&FAKE);
        assert!(!in_interrupt());
        enter_interrupt();
        enter_interrupt();
        exit_interrupt();
        assert!(in_interrupt());
        exit_interrupt();
        assert!(!in_interrupt());
    }

    #[test]
    fn nothing_without_ops() {
        let lock = Lock::new("test", 0);
        let node = LockNode::new();
        masked(|| assert!(!is_masked()));
        drop(lock.ilock(&node));
        enter_interrupt();
        assert!(!in_interrupt());
        exit_interrupt();
        assert!(!is_masked());
    }
}
