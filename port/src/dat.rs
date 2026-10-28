extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use bitflags::bitflags;
use core::ptr::{self, NonNull};
use core::result::Result;

use crate::mcslock::Lock;
use crate::mem::{PhysAddr, VirtRange};
use crate::pagealloc::PageAllocError;

pub struct Error {}

pub struct Chan {
    _offset: u64,
    _devoffset: u64,
    _typ: u16,
    _dev: u32,
    _mode: u16,
    _flag: u16,
    _qid: Qid,
    _fid: u32,
    _iounit: u32,
    // umh: Option<&Mutex<Mhead>>,
    // umqlock: Obviated by Mutex in umh?
    _umc: Option<NonNull<Chan>>,
    _uri: usize,
    _dri: usize,
    // dirrock: Option<&Mutex<*const ()>>,
    // rockqlock: Obviated by Mutex in dirrock?
    _nrock: usize,
    _mrock: usize,
    _ismtpt: bool,
    // mcp: *mut Mntcache,
    // mux: *mut Mnt,
    _aux: *mut (),
    _pgrpid: Qid,
    _mid: u32,
    // mchan: Arc<Chan>,
    _mqid: Qid,
    // path: *const Path,
}

pub struct Device {
    _dc: u32,
    _name: &'static str,
    _attached: bool,
}

bitflags! {
    pub struct Mode: u16 {
        const READ = 0;
        const WRITE = 1;
        const OEXEC = 2;
        const OTRUNC = 4;
        const OCEXEC = 5;
        const ORCLOSE = 6;
        const OEXCL = 12;
    }
}

pub trait Dev {
    fn reset();
    fn init();
    fn shutdown();
    fn attach(spec: &[u8]) -> Chan;
    fn walk(&self, c: &Chan, nc: &mut Chan, name: &[&[u8]]) -> Walkqid;
    fn stat(&self, c: &Chan, sb: &mut [u8]) -> Result<(), Error>;
    fn open(&mut self, c: &Chan, mode: u32) -> Chan;
    fn create(&mut self, c: &mut Chan, name: &[u8], mode: Mode, perms: u32);
    fn close(&mut self, c: Chan);
    fn read(&mut self, c: &mut Chan, buf: &mut [u8], offset: u64) -> Result<u64, Error>;
    fn bread(&mut self, c: &mut Chan, bnum: u64, offset: u64) -> Result<Block, Error>;
    fn write(&mut self, c: &mut Chan, buf: &[u8], offset: u64) -> Result<u64, Error>;
    fn bwrite(&mut self, c: &mut Chan, block: &Block, offset: u64) -> Result<u64, Error>;
    fn remove(&mut self, c: &mut Chan);
    fn wstat(&mut self, c: &mut Chan, sb: &[u8]) -> Result<(), Error>;
    fn power(&mut self, on: bool);
    fn config(&mut self /* other args */) -> Result<(), Error>;
}

pub struct Block {
    // TODO(cross): block linkage.
    _rp: usize,
    _wp: usize,
    _lim: *const u8,
    _base: *mut u8,
    _flag: u16,
    _checksum: u16,
    _magic: u32,
}

pub struct Qid {
    _path: u64,
    _vers: u32,
    _typ: u8,
}

pub struct Walkqid {
    _clone: Arc<Chan>,
    _qids: [Qid],
}

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
    /// The bss, which brk_ grows and shrinks (Plan 9's BSEG): every page from
    /// its start, rounded up, to its end is mapped.  None until it's set.
    pub(crate) bss: Option<VirtRange>,
    /// Set by pexit.  Empty means success, as in Plan 9.
    pub(crate) exit_status: Vec<u8>,
    /// The error string of the last syscall that failed, NUL-terminated, as
    /// Plan 9's syserrstr.  errstr swaps it with the caller's buffer.
    pub(crate) errstr: [u8; sys::ERRMAX],
    pub(crate) state: State,
    /// The tick at which its quantum runs out, set when it's dispatched.
    pub(crate) quantum_end: u64,
    /// The next process on the run queue.
    pub(crate) next: *mut Proc<A>,
}

impl<A: ProcArch> Proc<A> {
    /// The process's kernel stack.
    pub fn kstack(&self) -> &A::KStack {
        &self.kstack
    }
}

/// Ready processes, first in first out, linked through Proc::next.  Each is a
/// Box the queue owns, held as the raw pointer Box::into_raw gave it, so
/// queueing never allocates.
pub struct RunQueue<A: ProcArch> {
    head: *mut Proc<A>,
    tail: *mut Proc<A>,
}

impl<A: ProcArch> RunQueue<A> {
    pub const fn new() -> Self {
        RunQueue { head: ptr::null_mut(), tail: ptr::null_mut() }
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
