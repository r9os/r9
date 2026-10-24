//! Core-local interrupt masking and interrupt-context tracking.
//!
//! Any lock that is also taken in interrupt context (e.g. the console
//! lock) must be held with interrupts masked; otherwise an interrupt
//! arriving while the lock is held leaves the handler spinning on a
//! lock its own core can never release.  `IrqGuard` provides that
//! masking as an RAII guard.
//!
//! `in_interrupt` supports the complementary approach for subsystems
//! that interrupt context is simply forbidden to use (e.g. the
//! allocator): assert the invariant instead of masking around it.
//!
//! Masking is architecture-specific, so each arch registers its
//! implementation at early boot via `set_ops`, before enabling
//! interrupts (the pattern devcons uses for the Uart).  Until then, and
//! in hosted test builds where the mask instructions would be
//! privileged, `IrqGuard` is a no-op.

use core::marker::PhantomData;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

// Depth rather than a flag so nested exceptions stay counted.
// Core-local in spirit; needs to become per-core state under SMP.
static INTERRUPT_DEPTH: AtomicUsize = AtomicUsize::new(0);

/// Mark entry to interrupt context.  Called by the arch trap handler.
pub fn enter_interrupt() {
    INTERRUPT_DEPTH.fetch_add(1, Ordering::Relaxed);
}

/// Mark exit from interrupt context.  Called by the arch trap handler.
pub fn exit_interrupt() {
    INTERRUPT_DEPTH.fetch_sub(1, Ordering::Relaxed);
}

/// True while the current core is handling an interrupt or exception.
pub fn in_interrupt() -> bool {
    INTERRUPT_DEPTH.load(Ordering::Relaxed) > 0
}

/// Architecture hooks for masking interrupts on the current core.
/// `mask` masks interrupts and returns the previous interrupt state;
/// `restore` reinstates a state previously returned by `mask`.
pub struct IrqOps {
    pub mask: fn() -> u64,
    pub restore: fn(u64),
}

static IRQ_OPS: AtomicPtr<IrqOps> = AtomicPtr::new(ptr::null_mut());

/// Register the architecture's mask/restore implementation.  Call once
/// at early boot, before interrupts are first enabled.
pub fn set_ops(ops: &'static IrqOps) {
    IRQ_OPS.store(ops as *const IrqOps as *mut IrqOps, Ordering::Release);
}

fn ops() -> Option<&'static IrqOps> {
    let ops = IRQ_OPS.load(Ordering::Acquire);
    unsafe { ops.as_ref() }
}

/// Masks interrupts on the current core for its lifetime, restoring the
/// previous mask state on drop.  Nestable: taking a guard with
/// interrupts already masked (e.g. in interrupt context) is a no-op.
/// Create the guard before acquiring any lock shared with interrupt
/// context, and let it drop after the lock is released.
pub struct IrqGuard {
    saved: Option<u64>,
    // The saved state is core-local, so the guard must not move to
    // another core: !Send + !Sync.
    _not_send: PhantomData<*mut ()>,
}

impl IrqGuard {
    pub fn new() -> Self {
        Self { saved: ops().map(|ops| (ops.mask)()), _not_send: PhantomData }
    }
}

impl Default for IrqGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        if let (Some(saved), Some(ops)) = (self.saved, ops()) {
            (ops.restore)(saved);
        }
    }
}
