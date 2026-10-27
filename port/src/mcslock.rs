//! MCS locks
//!
//! Reference:
//!
//! John M. Mellor-Crummey and Michael L. Scott. 1991. Algorithms
//! for Scalable Synchronization on Shared Memory Multiprocessors.
//! ACM Transactions on Computer Systems 9, 1 (Feb. 1991), 21–65.
//! DOI: https://doi.org/10.1145/103727.103729

use core::cell::UnsafeCell;
use core::hint;
use core::marker::{Send, Sized, Sync};
use core::ops::{Deref, DerefMut};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use crate::irq;

/// Represents a node in the lock structure.  Note, is cacheline
/// aligned.
#[repr(align(64))]
pub struct LockNode {
    next: AtomicPtr<LockNode>,
    locked: AtomicBool,
}

impl LockNode {
    pub const fn new() -> LockNode {
        LockNode { next: AtomicPtr::new(ptr::null_mut()), locked: AtomicBool::new(false) }
    }
}

impl Default for LockNode {
    fn default() -> Self {
        Self::new()
    }
}

/// An MCS lock.
pub struct MCSLock {
    _name: &'static str,
    queue: AtomicPtr<LockNode>,
}

impl MCSLock {
    pub const fn new(name: &'static str) -> MCSLock {
        MCSLock { _name: name, queue: AtomicPtr::new(ptr::null_mut()) }
    }

    pub fn lock<'a>(&self, node: &'a LockNode) -> &'a LockNode {
        node.next.store(ptr::null_mut(), Ordering::Release);
        node.locked.store(false, Ordering::Release);
        let p = node as *const _ as *mut _;
        let predecessor = self.queue.swap(p, Ordering::AcqRel);
        if !predecessor.is_null() {
            let predecessor = unsafe { &*predecessor };
            node.locked.store(true, Ordering::Release);
            predecessor.next.store(p, Ordering::Release);
            while node.locked.load(Ordering::Acquire) {
                hint::spin_loop();
            }
        }
        node
    }

    /// Take the lock if it's free, without waiting, and without joining the
    /// queue if it isn't.
    pub fn try_lock<'a>(&self, node: &'a LockNode) -> Option<&'a LockNode> {
        node.next.store(ptr::null_mut(), Ordering::Release);
        node.locked.store(false, Ordering::Release);
        let p = node as *const _ as *mut _;
        self.queue
            .compare_exchange(ptr::null_mut(), p, Ordering::AcqRel, Ordering::Relaxed)
            .ok()
            .map(|_| node)
    }

    pub fn unlock(&self, node: &LockNode) {
        if node.next.load(Ordering::Acquire).is_null() {
            let p = node as *const _ as *mut _;
            if self
                .queue
                .compare_exchange_weak(p, ptr::null_mut(), Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
            while node.next.load(Ordering::Acquire).is_null() {
                hint::spin_loop();
            }
        }
        let next = node.next.load(Ordering::Acquire);
        let next = unsafe { &*next };
        next.locked.store(false, Ordering::Release);
    }
}

pub struct Lock<T: ?Sized> {
    lock: UnsafeCell<MCSLock>,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized> Send for Lock<T> {}
unsafe impl<T: ?Sized> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(name: &'static str, data: T) -> Lock<T> {
        Lock { lock: UnsafeCell::new(MCSLock::new(name)), data: UnsafeCell::new(data) }
    }

    pub fn lock<'a>(&'a self, node: &'a LockNode) -> LockGuard<'a, T> {
        let node = unsafe { &mut *self.lock.get() }.lock(node);
        self.guard(node, None)
    }

    /// Take the lock with IRQs masked until it's released, as Plan 9's
    /// ilock does, for data an interrupt handler also uses.  Masking first
    /// means a handler on this CPU can't spin on a lock the code it
    /// interrupted holds.
    pub fn ilock<'a>(&'a self, node: &'a LockNode) -> LockGuard<'a, T> {
        let node = unsafe { &mut *self.lock.get() }.lock(node);
        self.guard(node, irq::mask())
    }

    /// Take the lock if it's free, without waiting.
    pub fn try_lock<'a>(&'a self, node: &'a LockNode) -> Option<LockGuard<'a, T>> {
        let node = unsafe { &mut *self.lock.get() }.try_lock(node)?;
        Some(self.guard(node, None))
    }

    /// The guard for the lock, now that `node` holds it.
    fn guard<'a>(&'a self, node: &'a LockNode, irq: Option<irq::Saved>) -> LockGuard<'a, T> {
        LockGuard { lock: &self.lock, node, data: unsafe { &mut *self.data.get() }, irq }
    }
}

pub struct LockGuard<'a, T: ?Sized + 'a> {
    lock: &'a UnsafeCell<MCSLock>,
    node: &'a LockNode,
    data: &'a mut T,
    /// The IRQ state to restore once the lock is released, if ilock took it.
    irq: Option<irq::Saved>,
}
impl<T> Deref for LockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.data
    }
}

impl<T> DerefMut for LockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data
    }
}

impl<T: ?Sized> Drop for LockGuard<'_, T> {
    fn drop(&mut self) {
        unsafe { &mut *self.lock.get() }.unlock(self.node);
        irq::restore(self.irq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_lock_takes_only_a_free_lock() {
        let lock = MCSLock::new("test");
        let (a, b) = (LockNode::new(), LockNode::new());
        let held = lock.try_lock(&a).expect("free lock not taken");
        assert!(lock.try_lock(&b).is_none());
        // Had b joined the queue, unlocking would hand the lock to b.
        lock.unlock(held);
        let held = lock.try_lock(&b).expect("lock not free after unlock");
        lock.unlock(held);
    }

    #[test]
    fn try_lock_fails_with_a_waiter_queued() {
        let lock = MCSLock::new("test");
        let (holder, waiter, tryer) = (LockNode::new(), LockNode::new(), LockNode::new());
        let held = lock.lock(&holder);
        std::thread::scope(|s| {
            let waited = s.spawn(|| lock.unlock(lock.lock(&waiter)));
            // The waiter has queued once the queue's tail is its node.
            let waiter_ptr = (&raw const waiter).cast_mut();
            while lock.queue.load(Ordering::Acquire) != waiter_ptr {
                std::thread::yield_now();
            }
            assert!(lock.try_lock(&tryer).is_none());
            lock.unlock(held);
            waited.join().unwrap();
        });
        let held = lock.try_lock(&tryer).expect("lock not free after the waiter");
        lock.unlock(held);
    }

    #[test]
    fn lock_try_lock_guards_the_data() {
        let lock = Lock::new("test", 0);
        let (a, b) = (LockNode::new(), LockNode::new());
        {
            let mut g = lock.try_lock(&a).unwrap();
            *g += 1;
            assert!(lock.try_lock(&b).is_none());
        }
        assert_eq!(*lock.try_lock(&b).unwrap(), 1);
    }
}
