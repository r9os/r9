//! Process lifecycle, as Plan 9's port/proc.c.  There's no scheduler yet: a
//! process is run to completion from the scheduler context, and reaped there
//! when it exits.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::dat::{Proc, ProcArch};
use crate::pagealloc::{PageAllocError, free_physpage};

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// Why newproc failed.
#[derive(Debug, PartialEq)]
pub enum NewprocError {
    AddressSpace(PageAllocError),
    KernelStack(PageAllocError),
}

/// Create a process.  When run, its first context switches to its address
/// space and calls `start`, as Plan 9's kproc runs a function in a new
/// process.  `start` loads the process's program and enters user mode, and
/// returns only if that fails.
pub fn newproc<A: ProcArch>(start: fn()) -> Result<Proc<A>, NewprocError> {
    let pmmu = A::newpmmu().map_err(NewprocError::AddressSpace)?;
    let kstack = match A::kstack_alloc() {
        Ok(kstack) => kstack,
        Err(err) => {
            // SAFETY: nothing has used the new address space.
            unsafe { A::release(pmmu, |_| {}) };
            return Err(NewprocError::KernelStack(err));
        }
    };
    let label = A::new_context(proc_start::<A>, &kstack);
    let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
    Ok(Proc { pid, start, pmmu, kstack, label, exit_status: Vec::new() })
}

/// Run `p` until it exits, then reap it and return its exit status.
pub fn run<A: ProcArch>(mut p: Proc<A>) -> Vec<u8> {
    let up: *mut Proc<A> = &mut p;
    A::set_up(up);
    // SAFETY: the scheduler context is this CPU's, and p's label was made by
    // new_context.  This returns when p exits.  The label is reached through
    // up, as pexit saves into it.
    unsafe { A::swtch(A::sched(), &raw const (*up).label) };
    A::set_up(core::ptr::null_mut());
    // SAFETY: p has exited, so nothing runs on its stack or in its address
    // space.
    unsafe { reap(p) }
}

/// Exit the current process with `status`, switching back to the scheduler
/// context, which reaps it.
pub fn pexit<A: ProcArch>(status: &[u8]) -> ! {
    let up = A::up();
    assert!(!up.is_null(), "pexit: no current process");
    // SAFETY: up is the running process, set by run.
    unsafe {
        (*up).exit_status = status.to_vec();
        A::swtch(&mut (*up).label, A::sched());
    }
    unreachable!("pexit: an exited process was resumed")
}

/// The first context of every process: switch to its address space, and run
/// its start function.
extern "C" fn proc_start<A: ProcArch>() -> ! {
    let up = A::up();
    // SAFETY: run set up before switching here, and nothing refers to memory
    // through the empty address space being replaced.
    let start = unsafe {
        A::switch(&(*up).pmmu);
        (*up).start
    };
    start();
    pexit::<A>(b"start returned")
}

/// Free everything an exited process holds, and return its exit status.  A
/// process owns every page mapped in its address space, until there are
/// segments, so they're all freed.
///
/// # Safety
///
/// Nothing may still run on `p`'s kernel stack, or refer to memory through its
/// address space.
pub(crate) unsafe fn reap<A: ProcArch>(p: Proc<A>) -> Vec<u8> {
    let Proc { pmmu, kstack, exit_status, .. } = p;
    // SAFETY: the caller guarantees nothing uses the address space or the
    // stack.
    unsafe {
        A::release(pmmu, |pa| free_physpage(pa).expect("reap: couldn't free a process page"));
        A::kstack_free(kstack);
    }
    exit_status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::MMU;
    use crate::mem::PhysAddr;
    use std::cell::{Cell, RefCell};

    /// A fake architecture that records what it hands out and takes back.
    /// Tests run in parallel threads, so its state is per thread.
    struct Fake;

    thread_local! {
        static SPACES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        static STACKS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        static NEXT: Cell<u32> = const { Cell::new(1) };
        static NO_STACKS: Cell<bool> = const { Cell::new(false) };
    }

    fn next() -> u32 {
        NEXT.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        })
    }

    impl MMU for Fake {
        type Pmmu = u32;

        fn newpmmu() -> Result<u32, PageAllocError> {
            let id = next();
            SPACES.with(|s| s.borrow_mut().push(id));
            Ok(id)
        }

        fn put() {}
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

        unsafe fn swtch(_save: *mut (), _next: *const ()) {
            unreachable!("host tests don't switch contexts")
        }

        fn up() -> *mut Proc<Fake> {
            core::ptr::null_mut()
        }

        fn set_up(_p: *mut Proc<Fake>) {}

        fn sched() -> *mut () {
            core::ptr::null_mut()
        }
    }

    fn start() {}

    fn held() -> (usize, usize) {
        (SPACES.with(|s| s.borrow().len()), STACKS.with(|s| s.borrow().len()))
    }

    #[test]
    fn reaping_frees_what_newproc_took() {
        let p = newproc::<Fake>(start).expect("newproc failed");
        assert_eq!(held(), (1, 1));
        let q = newproc::<Fake>(start).expect("newproc failed");
        assert_ne!(p.pid, q.pid);
        assert_eq!(held(), (2, 2));

        let mut p = p;
        p.exit_status = b"bye".to_vec();
        // SAFETY: nothing runs in the fake process.
        assert_eq!(unsafe { reap(p) }, b"bye");
        // SAFETY: as above.
        unsafe { reap(q) };
        assert_eq!(held(), (0, 0));
    }

    #[test]
    fn newproc_without_a_kernel_stack_frees_its_address_space() {
        NO_STACKS.with(|n| n.set(true));
        let err = newproc::<Fake>(start).err();
        NO_STACKS.with(|n| n.set(false));
        assert_eq!(err, Some(NewprocError::KernelStack(PageAllocError::OutOfSpace)));
        assert_eq!(held(), (0, 0));
    }
}
