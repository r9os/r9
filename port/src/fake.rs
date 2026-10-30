//! A fake architecture for host tests.  It records the address spaces and
//! kernel stacks it hands out and takes back, has user memory with per-page
//! permissions, and a console that records what's written to it.  Tests run in
//! parallel threads, so its state is per thread.

use crate::dat::RunQueue;
use crate::dat::{Access, MMU, Perm, Proc, ProcArch, PutError, UserRegs};
use crate::console::{self, Uart};
use crate::mcslock::Lock;
use crate::mem::{PAGE_SIZE_4K, PhysAddr};
use crate::pagealloc::PageAllocError;
use std::cell::{Cell, RefCell};

pub(crate) struct Fake;

/// The fake's user address space ends here, above any address the host's
/// heap uses.
pub(crate) const UADDRTOP: usize = 1 << 47;

/// The fake's user stack is just below UADDRTOP.
pub(crate) const USTKTOP: usize = UADDRTOP;

thread_local! {
    static SPACES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    static STACKS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<u32> = const { Cell::new(1) };
    /// Set to make kstack_alloc fail.
    pub(crate) static NO_STACKS: Cell<bool> = const { Cell::new(false) };
    /// The user pages: each page's base, and the access it allows (Read
    /// allows reading only, Write allows both).
    static PAGES: RefCell<Vec<(usize, Access)>> = const { RefCell::new(Vec::new()) };
    static CONS_OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Whether IRQs were masked as each byte was written to the console.
    static CONS_MASKED: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    /// The current process, set by set_up.
    static UP: Cell<*mut Proc<Fake>> = const { Cell::new(core::ptr::null_mut()) };
    static RUN_QUEUE: &'static Lock<RunQueue<Fake>> = Box::leak(Box::new(Lock::new("run_queue", RunQueue::new())));
    /// What put has mapped and unput hasn't unmapped: each page's address,
    /// physical page and permission.
    static MAPPED: RefCell<Vec<(usize, PhysAddr, Perm)>> = const { RefCell::new(Vec::new()) };
}

/// The pages put has mapped, in the order it mapped them.
pub(crate) fn mapped() -> Vec<(usize, PhysAddr, Perm)> {
    MAPPED.with(|m| m.borrow().clone())
}

/// A UART that appends to the writing thread's CONS_OUT.
struct FakeUart;

impl Uart for FakeUart {
    fn putb(&self, b: u8) {
        CONS_OUT.with(|o| o.borrow_mut().push(b));
        CONS_MASKED.with(|m| m.borrow_mut().push(crate::irq::tests::is_masked()));
    }
}

/// Make the console FakeUart, and clear this thread's output.  The console is
/// shared by every thread, but each thread's output is its own.
pub(crate) fn fake_cons() {
    console::set_uart(Box::leak(Box::new(FakeUart)));
    CONS_OUT.with(|o| o.borrow_mut().clear());
    CONS_MASKED.with(|m| m.borrow_mut().clear());
}

/// Whether IRQs were masked as each byte was written to the console, since
/// the last call.
pub(crate) fn cons_masked() -> Vec<bool> {
    CONS_MASKED.with(|m| core::mem::take(&mut *m.borrow_mut()))
}

/// What this thread has written to the console since the last call.
pub(crate) fn cons_output() -> Vec<u8> {
    CONS_OUT.with(|o| core::mem::take(&mut *o.borrow_mut()))
}

fn next() -> u32 {
    NEXT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

/// The address spaces and kernel stacks currently held.
pub(crate) fn held() -> (usize, usize) {
    (SPACES.with(|s| s.borrow().len()), STACKS.with(|s| s.borrow().len()))
}

#[repr(C, align(4096))]
pub(crate) struct Mem(pub(crate) [u8; 3 * PAGE_SIZE_4K]);

impl Mem {
    pub(crate) fn base(&self) -> usize {
        self.0.as_ptr().addr()
    }
}

/// Three pages of user memory: read-write, read-only, then unmapped.
pub(crate) fn user_mem() -> Box<Mem> {
    let mem = Box::new(Mem([0; 3 * PAGE_SIZE_4K]));
    let base = mem.base();
    assert!(base + mem.0.len() <= UADDRTOP);
    PAGES.with(|p| {
        *p.borrow_mut() = vec![(base, Access::Write), (base + PAGE_SIZE_4K, Access::Read)]
    });
    mem
}

impl MMU for Fake {
    type Pmmu = u32;

    const UADDRTOP: usize = UADDRTOP;
    const UTZERO: usize = 0x10000;
    const USTKTOP: usize = USTKTOP;
    const USTKSIZE: usize = 64 * 1024;

    fn newpmmu() -> Result<u32, PageAllocError> {
        let id = next();
        SPACES.with(|s| s.borrow_mut().push(id));
        Ok(id)
    }

    fn user_page_ok(va: usize, access: Access) -> bool {
        let base = va & !(PAGE_SIZE_4K - 1);
        PAGES.with(|p| {
            p.borrow()
                .iter()
                .any(|&(b, a)| b == base && (access == Access::Read || a == Access::Write))
        })
    }

    fn put(va: usize, pa: PhysAddr, perm: Perm) -> Result<(), PutError> {
        MAPPED.with(|m| {
            let mut m = m.borrow_mut();
            if m.iter().any(|&(v, _, _)| v == va) {
                return Err(PutError::AlreadyMapped);
            }
            m.push((va, pa, perm));
            Ok(())
        })
    }

    fn unput(va: usize) -> Option<PhysAddr> {
        MAPPED.with(|m| {
            let mut m = m.borrow_mut();
            let i = m.iter().position(|&(v, _, _)| v == va)?;
            Some(m.remove(i).1)
        })
    }

    fn kmap<R>(_pa: PhysAddr, _f: impl FnOnce(*mut u8) -> R) -> R {
        unreachable!("host tests have no physical pages to map")
    }

    fn flush() {}
    fn flushva(_ptr: *const ()) {}

    unsafe fn switch(_pmmu: &u32) {}

    unsafe fn release(pmmu: u32, _free_leaf: impl FnMut(PhysAddr)) {
        SPACES.with(|s| s.borrow_mut().retain(|&id| id != pmmu));
    }
}

impl ProcArch for Fake {
    type Label = ();
    type KStack = u32;

    fn kstack_alloc() -> Result<u32, PageAllocError> {
        if NO_STACKS.with(Cell::get) {
            return Err(PageAllocError::OutOfSpace);
        }
        let id = next();
        STACKS.with(|s| s.borrow_mut().push(id));
        Ok(id)
    }

    unsafe fn kstack_free(stack: u32) {
        STACKS.with(|s| s.borrow_mut().retain(|&id| id != stack));
    }

    fn new_context(_entry: extern "C" fn() -> !, _stack: &u32) {}

    fn fork_context(_parent: &u32, _child: &u32) {}

    unsafe fn enter_user(_regs: &UserRegs) -> ! {
        unreachable!("host tests don't enter user mode")
    }

    unsafe fn swtch(_save: *mut (), _next: *const ()) {
        unreachable!("host tests don't switch contexts")
    }

    fn up() -> *mut Proc<Fake> {
        UP.with(Cell::get)
    }

    fn set_up(p: *mut Proc<Fake>) {
        UP.with(|up| up.set(p));
    }

    fn sched() -> *mut () {
        core::ptr::null_mut()
    }

    type Fp = ();

    fn fp_save(_fp: &mut ()) {}

    fn fp_restore(_fp: &()) {}

    fn idle() {
        unreachable!("host tests don't idle")
    }

    fn run_queue() -> &'static Lock<RunQueue<Fake>> {
        RUN_QUEUE.with(|q| *q)
    }
}
