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

/// Represents a node in the lock structure.  Note, is cacheline
/// aligned.
#[repr(align(64))]
pub struct LockNode {
    next: AtomicPtr<LockNode>,
    locked: AtomicBool,
}

impl LockNode {
    pub const fn new() -> LockNode {
        LockNode {
            next: AtomicPtr::new(ptr::null_mut()),
            locked: AtomicBool::new(false),
        }
    }
}

/// An MCS lock.
pub struct MCSLock {
    name: &'static str,
    queue: AtomicPtr<LockNode>,
}

impl MCSLock {
    pub const fn new(name: &'static str) -> MCSLock {
        MCSLock {
            name,
            queue: AtomicPtr::new(ptr::null_mut()),
        }
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
        Lock {
            lock: UnsafeCell::new(MCSLock::new(name)),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock<'a>(&'a self, node: &'a LockNode) -> LockGuard<'a, T> {
        let node = unsafe { &mut *self.lock.get() }.lock(node);
        LockGuard {
            lock: &self.lock,
            node,
            data: unsafe { &mut *self.data.get() },
        }
    }
}

pub struct LockGuard<'a, T: ?Sized + 'a> {
    lock: &'a UnsafeCell<MCSLock>,
    node: &'a LockNode,
    data: &'a mut T,
}
impl<'a, T> Deref for LockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.data
    }
}

impl<'a, T> DerefMut for LockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data
    }
}

impl<'a, T: ?Sized> Drop for LockGuard<'a, T> {
    fn drop(&mut self) {
        unsafe { &mut *self.lock.get() }.unlock(self.node);
    }
}
