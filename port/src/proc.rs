//! Processes, and the scheduler, after Plan 9's port/proc.c.  Every switch
//! goes through the scheduler's context: a process switches to it to yield
//! or exit, and schedule, running there, re-queues or reaps it, then picks the
//! next.  So a process is reaped off its own stack, and re-queued only once
//! its context is saved.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::dat::{Proc, ProcArch, State};
use crate::irq;
use crate::mcslock::LockNode;
use crate::pagealloc::{PageAllocError, free_physpage};
use crate::timer;

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// How many ticks a process runs before it can be preempted: 100 ms, as Plan
/// 9's HZ/10.
const QUANTUM: u64 = timer::HZ / 10;

/// Why newproc failed.
#[derive(Debug, PartialEq)]
pub enum NewprocError {
    AddressSpace(PageAllocError),
    KernelStack(PageAllocError),
}

/// Create a process.  When it's first run, it calls `start`, as Plan 9's
/// kproc runs a function in a new process.  `start` loads the process's
/// program and enters user mode, and returns only if that fails.
pub fn newproc<A: ProcArch>(start: fn()) -> Result<Box<Proc<A>>, NewprocError> {
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
    Ok(Box::new(Proc {
        pid,
        start,
        pmmu,
        kstack,
        label,
        bss: None,
        exit_status: Vec::new(),
        errstr: [0; sys::ERRMAX],
        state: State::Ready,
        fp: A::Fp::default(),
        quantum_end: 0,
        next: ptr::null_mut(),
    }))
}

/// Queue `p` to run.
pub fn ready<A: ProcArch>(mut p: Box<Proc<A>>) {
    p.state = State::Ready;
    let node = LockNode::new();
    A::run_queue().ilock(&node).push(p);
}

fn dequeue<A: ProcArch>() -> Option<Box<Proc<A>>> {
    let node = LockNode::new();
    A::run_queue().ilock(&node).pop()
}

/// Run ready processes until there are none, in the caller's context, which
/// becomes this CPU's scheduler context, and return each one's pid and exit
/// status, in the order they exited.
pub fn schedule<A: ProcArch>() -> Vec<(u64, Vec<u8>)> {
    let mut exits = Vec::new();
    while let Some(mut p) = dequeue::<A>() {
        p.state = State::Running;
        p.quantum_end = timer::ticks() + QUANTUM;
        // SAFETY: nothing here refers to memory through the address space
        // being replaced.
        unsafe { A::switch(&p.pmmu) };
        // The kernel doesn't use the FP/SIMD registers, so they hold a
        // process's own until the scheduler loads the next one's.
        A::fp_restore(&p.fp);
        // While it runs, the process is reached only through up, so the Box
        // gives way to its raw pointer until it switches back.
        let up = Box::into_raw(p);
        A::set_up(up);
        // A process switches back from inside an exception, which masks
        // IRQs, so save this context's state first and restore it after, as
        // Plan 9's sched brackets its switch with splhi and splx.
        let saved = irq::mask();
        // SAFETY: the scheduler context is this CPU's, and up's label was made
        // by new_context or saved by swtch.
        unsafe { A::swtch(A::sched(), &raw const (*up).label) };
        irq::restore(saved);
        A::set_up(ptr::null_mut());
        // SAFETY: up is from Box::into_raw, and the process has switched back,
        // so nothing else refers to it.
        let mut p = unsafe { Box::from_raw(up) };
        if p.state != State::Exited {
            A::fp_save(&mut p.fp);
        }
        match p.state {
            State::Running => ready(p),
            State::Exited => {
                let pid = p.pid;
                // SAFETY: p has exited, so nothing runs on its stack or in its
                // address space.
                exits.push((pid, unsafe { reap(*p) }));
            }
            State::Ready => panic!("schedule: process {} came back ready", p.pid),
        }
    }
    exits
}

/// Run `p` until it exits, and return its exit status.  Nothing else may be
/// ready, so only `p` runs.
pub fn run<A: ProcArch>(p: Box<Proc<A>>) -> Vec<u8> {
    let node = LockNode::new();
    assert!(A::run_queue().ilock(&node).is_empty(), "run: other processes are ready");
    let pid = p.pid;
    ready(p);
    let exits = schedule::<A>();
    exits.into_iter().find(|&(p, _)| p == pid).map(|(_, status)| status).expect("run: no exit")
}

/// Give up the CPU, staying ready, as Plan 9's sched does for a running
/// process.
pub fn yield_now<A: ProcArch>() {
    let up = A::up();
    assert!(!up.is_null(), "yield_now: no current process");
    // SAFETY: up is the running process, set by schedule, which resumes it
    // here.
    unsafe { A::swtch(&mut (*up).label, A::sched()) };
}

/// Whether the running process has used its quantum while another is ready,
/// as Plan 9's hzsched decides.
pub fn preempt_due<A: ProcArch>() -> bool {
    let up = A::up();
    // SAFETY: up is null or the running process, set by schedule.
    let Some(up) = (unsafe { up.as_ref() }) else {
        return false;
    };
    let node = LockNode::new();
    timer::ticks() >= up.quantum_end && !A::run_queue().ilock(&node).is_empty()
}

/// Exit the current process with `status`, switching back to the scheduler
/// context, which reaps it.
pub fn pexit<A: ProcArch>(status: &[u8]) -> ! {
    let up = A::up();
    assert!(!up.is_null(), "pexit: no current process");
    // SAFETY: up is the running process, set by schedule.
    unsafe {
        (*up).exit_status = status.to_vec();
        (*up).state = State::Exited;
        A::swtch(&mut (*up).label, A::sched());
    }
    unreachable!("pexit: an exited process was resumed")
}

/// The first context of every process, in its address space: run its start
/// function.
extern "C" fn proc_start<A: ProcArch>() -> ! {
    // SAFETY: schedule set up before switching here.
    let start = unsafe { (*A::up()).start };
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
    use crate::dat::ProcArch;
    use crate::fake::{Fake, NO_STACKS, held};

    fn start() {}

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
        assert_eq!(unsafe { reap(*p) }, b"bye");
        // SAFETY: as above.
        unsafe { reap(*q) };
        assert_eq!(held(), (0, 0));
    }

    /// Take the next ready process.
    fn next() -> Option<Box<Proc<Fake>>> {
        dequeue::<Fake>()
    }

    #[test]
    fn the_run_queue_is_first_in_first_out() {
        let procs: Vec<_> = (0..3).map(|_| newproc::<Fake>(start).unwrap()).collect();
        let pids: Vec<_> = procs.iter().map(|p| p.pid).collect();
        procs.into_iter().for_each(ready);
        let order: Vec<_> = core::iter::from_fn(next)
            .map(|p| {
                let pid = p.pid;
                // SAFETY: nothing runs in the fake process.
                unsafe { reap(*p) };
                pid
            })
            .collect();
        assert_eq!(order, pids);
        assert_eq!(held(), (0, 0));
    }

    #[test]
    fn preemption_is_due_after_the_quantum_with_another_ready() {
        let mut p = newproc::<Fake>(start).unwrap();
        assert!(!preempt_due::<Fake>(), "nothing is running");
        Fake::set_up(&mut *p);
        p.quantum_end = timer::ticks();
        assert!(!preempt_due::<Fake>(), "nothing else is ready");
        ready(newproc::<Fake>(start).unwrap());
        assert!(preempt_due::<Fake>());
        p.quantum_end = timer::ticks() + 1;
        assert!(!preempt_due::<Fake>(), "the quantum isn't used");
        Fake::set_up(ptr::null_mut());
        // SAFETY: nothing runs in the fake processes.
        unsafe {
            reap(*next().unwrap());
            reap(*p);
        }
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
