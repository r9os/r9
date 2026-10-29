extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr;
use core::result::Result;

use crate::chan::Fds;
use crate::mcslock::Lock;
use crate::mem::{PhysAddr, VirtRange};
use crate::pagealloc::PageAllocError;
use crate::timer::Timer;

/// A kind of access to memory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Access {
    Read,
    Write,
}

/// What a user page may be used for.  Text is read-only and executable,
/// rodata is read-only, and data is read-write, and neither is ever
/// executable, so no page is both writable and executable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Perm {
    Text,
    Rodata,
    Data,
}

/// Why MMU::put couldn't map a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PutError {
    /// Something is already mapped at the address.
    AlreadyMapped,
    /// Memory for the page or its page tables couldn't be allocated.
    NoMemory,
}

/// The architecture's MMU, for port code.  Operations take the architecture's
/// per-process MMU state, rather than a Proc, so that the trait and Proc don't
/// depend on each other.  flush and flushva are placeholders, with no callers
/// yet.
pub trait MMU {
    /// Per-process MMU state (Plan 9's PMMU): a user address space.
    type Pmmu;

    /// The end of the user address space.
    const UADDRTOP: usize;

    /// The start of user text.
    const UTZERO: usize;

    /// The top of the user stack, and its size.
    const USTKTOP: usize;
    const USTKSIZE: usize;

    /// Create a new user address space, with nothing mapped.
    fn newpmmu() -> Result<Self::Pmmu, PageAllocError>;

    /// Whether user mode could make `access` to the page containing `va`, in
    /// the current address space.
    fn user_page_ok(va: usize, access: Access) -> bool;

    /// Map the 4KiB page at `pa` at `va`, with `perm`, in the current user
    /// address space.  Text is first made visible to instruction fetch, as
    /// Plan 9's putmmu does for pages marked PG_TXTFLUSH.
    fn put(va: usize, pa: PhysAddr, perm: Perm) -> Result<(), PutError>;

    /// Unmap the 4KiB page at `va` in the current user address space, and
    /// return it, or None if nothing was mapped there.  The caller now owns
    /// the page.
    fn unput(va: usize) -> Option<PhysAddr>;

    /// Run `f` with the 4KiB page at `pa` mapped at the address it's given.
    fn kmap<R>(pa: PhysAddr, f: impl FnOnce(*mut u8) -> R) -> R;

    fn flush();
    fn flushva(ptr: *const ());

    /// Make `pmmu` the current user address space.
    ///
    /// # Safety
    ///
    /// Nothing may still refer to memory through the address space being
    /// replaced.
    unsafe fn switch(pmmu: &Self::Pmmu);

    /// Free `pmmu`'s page tables, and pass each page mapped in it to
    /// `free_leaf`, which frees it if the caller owns it.
    ///
    /// # Safety
    ///
    /// Nothing may still refer to memory through `pmmu`.
    unsafe fn release(pmmu: Self::Pmmu, free_leaf: impl FnMut(PhysAddr));
}

/// What a process needs from the architecture, besides its MMU.
pub trait ProcArch: MMU + Sized + 'static {
    /// A saved kernel context, as swtch saves and restores.
    type Label;

    /// A kernel stack.
    type KStack;

    /// A process's FP/SIMD registers, saved while it isn't running.  Its
    /// default is the state a new process starts with.
    type Fp: Default;

    /// Allocate a kernel stack.
    fn kstack_alloc() -> Result<Self::KStack, PageAllocError>;

    /// Free a kernel stack.
    ///
    /// # Safety
    ///
    /// No context may be running on `stack`, or be switched to on it later.
    unsafe fn kstack_free(stack: Self::KStack);

    /// A context that, when switched to, starts running `entry` on `stack`.
    fn new_context(entry: extern "C" fn() -> !, stack: &Self::KStack) -> Self::Label;

    /// Save the current context in `save`, and resume the one in `next`.
    ///
    /// # Safety
    ///
    /// `save` must be valid for writes, and `next` must hold a context saved
    /// by swtch or made by new_context.
    unsafe fn swtch(save: *mut Self::Label, next: *const Self::Label);

    /// This CPU's current process (Plan 9's `up`), or null if there's none.
    fn up() -> *mut Proc<Self>;

    /// Set this CPU's current process.
    fn set_up(p: *mut Proc<Self>);

    /// This CPU's scheduler context (Plan 9's `m->sched`), which a process
    /// switches back to when it yields or exits.
    fn sched() -> *mut Self::Label;

    /// Save this CPU's FP/SIMD registers into `fp`.
    fn fp_save(fp: &mut Self::Fp);

    /// Load this CPU's FP/SIMD registers from `fp`.
    fn fp_restore(fp: &Self::Fp);

    /// Wait for an interrupt, with IRQs masked: a pending IRQ ends the wait
    /// anyway, and the caller unmasks to take it.
    fn idle();

    /// The run queue.  It's the architecture's to hold, since Proc is
    /// generic over it.
    fn run_queue() -> &'static Lock<RunQueue<Self>>;
}

/// Where a process is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// On the run queue.
    Ready,
    /// Running, or switched back to the scheduler to be re-queued.
    Running,
    /// Asleep on a Rendez, until a wakeup.
    Sleeping,
    /// Exited, to be reaped.
    Exited,
}

/// A process.  See port::proc for its lifecycle.
pub struct Proc<A: ProcArch> {
    pub pid: u64,
    /// Run in the process's own context, with its address space active, to
    /// load its program and enter user mode.  It returns only on failure.
    pub(crate) start: fn(),
    pub(crate) pmmu: A::Pmmu,
    pub(crate) kstack: A::KStack,
    pub(crate) label: A::Label,
    /// Its text, rodata, data and stack, as Plan 9's seg[]: every page of
    /// each is mapped.
    pub(crate) segments: Vec<UserSegment>,
    /// The bss, which brk_ grows and shrinks (Plan 9's BSEG): every page from
    /// its start, rounded up, to its end is mapped.  None until it's set.
    pub(crate) bss: Option<VirtRange>,
    /// The process's file descriptors, as Plan 9's up->fgrp, which it may
    /// share with other processes.  newproc opens the console on 0, 1 and 2.
    /// None once it has exited, so pexit can let them go without allocating.
    pub(crate) fds: Option<Arc<Fds>>,
    /// Set by pexit.  Empty means success, as in Plan 9.
    pub(crate) exit_status: Vec<u8>,
    /// The error string of the last syscall that failed, NUL-terminated, as
    /// Plan 9's syserrstr.  errstr swaps it with the caller's buffer.
    pub(crate) errstr: [u8; sys::ERRMAX],
    pub(crate) state: State,
    /// Its FP/SIMD registers, while it isn't running.
    pub(crate) fp: A::Fp,
    /// The tick at which its quantum runs out, set when it's dispatched.
    pub(crate) quantum_end: u64,
    /// The next process on the run queue.
    pub(crate) next: *mut Proc<A>,
    /// The process's own Rendez, for SLEEP, as Plan 9's up->sleep.
    pub(crate) sleep: Rendez,
    /// Its tsleep's timeout, as Plan 9's up->t: added and removed on every
    /// tsleep, and removed before the process is reaped.
    pub(crate) timer: Timer,
    /// The Rendez its tsleep's timeout should wake, cleared when the timeout
    /// fires, as Plan 9's up->trend.
    pub(crate) trend: *const Rendez,
    /// Its program's name, as Plan 9's up->text: at most KNAMELEN - 1 bytes,
    /// set by exec.
    pub(crate) text: Vec<u8>,
    /// Its children's exits, which its children share.
    pub(crate) waitq: Arc<WaitQueue>,
    /// Its parent's wait queue, which its exit goes to, or None for a
    /// process the kernel started.
    pub(crate) parent: Option<Arc<WaitQueue>>,
}

/// The longest process text, with its NUL, as Plan 9's KNAMELEN.
pub const KNAMELEN: usize = 28;

/// A child's exit, as Plan 9's Waitmsg: its pid, and its message, empty if
/// it succeeded.
#[derive(Clone, Debug, PartialEq)]
pub struct Waitmsg {
    pub pid: u64,
    pub msg: Vec<u8>,
}

/// The most exits a wait queue holds, as Plan 9's pexit bounds a parent's
/// waitq: past it, an exit is dropped, so a parent that never waits can't
/// collect them without limit.
pub(crate) const MAXWAIT: usize = 128;

/// A process's children's exits, as Plan 9's waitq, nchild and waitr: shared,
/// through an Arc, by the process and each child it starts, which queues its
/// exit here, so there's no process table to find a parent in.  Once its
/// process has exited, nothing more is queued, and it goes when its last
/// child does.
pub struct WaitQueue {
    /// Taken with ilock, since a waiter's sleep checks it with IRQs masked.
    pub(crate) state: Lock<WaitState>,
    /// Where the process waits for an exit.
    pub(crate) waitr: Rendez,
}

pub(crate) struct WaitState {
    /// Its children's exits, newest first, as Plan 9's waitq.
    pub(crate) exits: VecDeque<Waitmsg>,
    /// How many of its children are still alive, as Plan 9's nchild.
    pub(crate) nchild: usize,
    /// Whether its process is alive, as Plan 9's pexit checks its parent is.
    pub(crate) alive: bool,
    /// Whether its process is waiting, as Plan 9's qwaitr: one waiter at a
    /// time.
    pub(crate) waiting: bool,
}

impl WaitQueue {
    pub(crate) fn new() -> WaitQueue {
        WaitQueue {
            state: Lock::new(
                "waitq",
                WaitState { exits: VecDeque::new(), nchild: 0, alive: true, waiting: false },
            ),
            waitr: Rendez::new(),
        }
    }
}

impl<A: ProcArch> Proc<A> {
    /// The process's kernel stack.
    pub fn kstack(&self) -> &A::KStack {
        &self.kstack
    }

    /// The process's segments, but for its bss.
    pub fn segments(&self) -> &[UserSegment] {
        &self.segments
    }

    /// The process's bss, if it's set.
    pub fn bss(&self) -> Option<VirtRange> {
        self.bss
    }

    /// The process's file descriptors.
    pub(crate) fn fds(&self) -> &Arc<Fds> {
        self.fds.as_ref().expect("a process that has exited has no descriptors")
    }
}

/// A segment of a process's memory, as Plan 9's Segment: whole pages, all
/// mapped with `perm`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UserSegment {
    pub range: VirtRange,
    pub perm: Perm,
}

/// Ready processes, first in first out, linked through Proc::next.  Each is a
/// Box the queue owns, held as the raw pointer Box::into_raw gave it, so
/// queueing never allocates.
pub struct RunQueue<A: ProcArch> {
    head: *mut Proc<A>,
    tail: *mut Proc<A>,
    /// How many processes are asleep, so the scheduler can tell nothing to
    /// run yet from nothing left.
    pub(crate) asleep: usize,
}

impl<A: ProcArch> RunQueue<A> {
    pub const fn new() -> Self {
        RunQueue { head: ptr::null_mut(), tail: ptr::null_mut(), asleep: 0 }
    }

    pub(crate) fn push(&mut self, p: Box<Proc<A>>) {
        let p = Box::into_raw(p);
        // An empty queue is a case of its own: pointing tail at the last
        // link, starting at head, would make the queue refer to itself, which
        // a static can't be built with.
        // SAFETY: p is from Box::into_raw, and so is tail, if it's not null.
        unsafe {
            (*p).next = ptr::null_mut();
            match self.tail.as_mut() {
                Some(tail) => tail.next = p,
                None => self.head = p,
            }
        }
        self.tail = p;
    }

    pub(crate) fn pop(&mut self) -> Option<Box<Proc<A>>> {
        if self.head.is_null() {
            return None;
        }
        // SAFETY: head is from Box::into_raw, and is taken off the queue
        // before it's turned back into a Box, so it's owned once.
        let p = unsafe { Box::from_raw(self.head) };
        self.head = p.next;
        if self.head.is_null() {
            self.tail = ptr::null_mut();
        }
        Some(p)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.head.is_null()
    }
}

impl<A: ProcArch> Default for RunQueue<A> {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a process waits to be woken, as Plan 9's Rendez: the one process
/// asleep here, or null.  While it sleeps, the process is owned through this
/// pointer, from Box::into_raw, as the run queue owns a ready one.  It's a
/// Proc<A> for the one architecture the kernel runs, untyped so that code
/// that isn't generic over the architecture, such as a device, can hold a
/// Rendez.
pub struct Rendez {
    pub(crate) sleeper: Lock<*mut ()>,
}

impl Rendez {
    pub const fn new() -> Self {
        Rendez { sleeper: Lock::new("rendez", ptr::null_mut()) }
    }
}

impl Default for Rendez {
    fn default() -> Self {
        Self::new()
    }
}
