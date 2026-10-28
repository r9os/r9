//! Processes, and the scheduler, after Plan 9's port/proc.c.  Every switch
//! goes through the scheduler's context: a process switches to it to yield
//! or exit, and schedule, running there, re-queues or reaps it, then picks the
//! next.  So a process is reaped off its own stack, and re-queued only once
//! its context is saved.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use crate::dat::{Proc, ProcArch, Rendez, State};
use crate::irq;
use crate::mcslock::LockNode;
use crate::pagealloc::{PageAllocError, free_physpage};
use crate::timer::{self, Mode, Timer};

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
        sleep: Rendez::new(),
        timer: Timer::new(Mode::Relative(Duration::from_millis(1)), |_| {}),
        trend: ptr::null(),
    }))
}

/// Queue `p` to run.
pub fn ready<A: ProcArch>(mut p: Box<Proc<A>>) {
    p.state = State::Ready;
    let node = LockNode::new();
    A::run_queue().ilock(&node).push(p);
}

/// Run ready processes until none is ready or asleep, in the caller's
/// context, which becomes this CPU's scheduler context, and return each one's
/// pid and exit status, in the order they exited.  With nothing ready but
/// something asleep, wait for the interrupt that wakes it.
pub fn schedule<A: ProcArch>() -> Vec<(u64, Vec<u8>)> {
    let mut exits = Vec::new();
    loop {
        // A process switches back from inside an exception, which masks
        // IRQs, so save this context's state, and restore it only once the
        // process is re-queued or handed to its Rendez: an IRQ in between
        // could wake a process the scheduler still holds.  Checking for work
        // and waiting with IRQs masked means a wakeup between the two isn't
        // lost.
        let saved = irq::mask();
        let node = LockNode::new();
        let mut q = A::run_queue().ilock(&node);
        let (next, asleep) = (q.pop(), q.asleep);
        drop(q);
        let Some(p) = next else {
            if asleep == 0 {
                irq::restore(saved);
                return exits;
            }
            A::idle();
            irq::restore(saved);
            continue;
        };
        let exited = dispatch(p);
        irq::restore(saved);
        // Reaping a whole address space can take a while, and an exited
        // process is on no Rendez, so it's done with IRQs unmasked.
        if let Some(p) = exited {
            let pid = p.pid;
            // SAFETY: p has exited, so nothing runs on its stack or in its
            // address space.
            exits.push((pid, unsafe { reap(*p) }));
        }
    }
}

/// Run `p` until it switches back, then re-queue it, or hand it to the
/// Rendez it sleeps on, or return it if it exited.
fn dispatch<A: ProcArch>(mut p: Box<Proc<A>>) -> Option<Box<Proc<A>>> {
    p.state = State::Running;
    p.quantum_end = timer::ticks() + QUANTUM;
    // SAFETY: nothing here refers to memory through the address space being
    // replaced.
    unsafe { A::switch(&p.pmmu) };
    // The kernel doesn't use the FP/SIMD registers, so they hold a process's
    // own until the scheduler loads the next one's.
    A::fp_restore(&p.fp);
    // While it runs, the process is reached only through up, so the Box gives
    // way to its raw pointer until it switches back.
    let up = Box::into_raw(p);
    A::set_up(up);
    // SAFETY: the scheduler context is this CPU's, and up's label was made by
    // new_context or saved by swtch.
    unsafe { A::swtch(A::sched(), &raw const (*up).label) };
    A::set_up(ptr::null_mut());
    // SAFETY: up is from Box::into_raw, and the process has switched back, so
    // nothing else refers to it.
    let mut p = unsafe { Box::from_raw(up) };
    if p.state != State::Exited {
        A::fp_save(&mut p.fp);
    }
    match p.state {
        State::Running => ready(p),
        State::Sleeping => {
            // The Rendez it sleeps on already holds its pointer, and owns it
            // from here.
            let node = LockNode::new();
            A::run_queue().ilock(&node).asleep += 1;
            Box::leak(p);
        }
        State::Exited => return Some(p),
        State::Ready => panic!("schedule: process {} came back ready", p.pid),
    }
    None
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

/// Sleep on `r` until `cond` holds, as Plan 9's sleep.  `cond` is checked
/// with `r` locked, so a wakeup after it's made true can't be missed, and
/// again each time the process is woken.  IRQs stay masked until the process
/// is back in the scheduler, which takes ownership of it before anything can
/// wake it.
pub fn sleep<A: ProcArch>(r: &Rendez<A>, cond: impl Fn() -> bool) {
    irq::masked(|| {
        loop {
            let node = LockNode::new();
            let mut sleeper = r.sleeper.ilock(&node);
            if cond() {
                return;
            }
            assert!(sleeper.is_null(), "sleep: another process is asleep here");
            let up = A::up();
            assert!(!up.is_null(), "sleep: no current process");
            // SAFETY: up is the running process, set by schedule, which
            // resumes it here once it's woken.  It's marked asleep before
            // it's published on r, so a wakeup never sees it running.
            unsafe { (*up).state = State::Sleeping };
            *sleeper = up;
            drop(sleeper);
            // SAFETY: as above.
            unsafe { A::swtch(&mut (*up).label, A::sched()) };
        }
    })
}

/// Wake the process asleep on `r`, if there is one, as Plan 9's wakeup, and
/// return whether there was.  It doesn't allocate, so it can be called from
/// an interrupt handler.
pub fn wakeup<A: ProcArch>(r: &Rendez<A>) -> bool {
    let node = LockNode::new();
    let p = core::mem::replace(&mut *r.sleeper.ilock(&node), ptr::null_mut());
    if p.is_null() {
        return false;
    }
    // SAFETY: the Rendez owned p, from Box::into_raw, and has just given it up.
    let mut p = unsafe { Box::from_raw(p) };
    p.state = State::Ready;
    let mut q = A::run_queue().ilock(&node);
    // The scheduler counted p before it unmasked IRQs, and on one CPU nothing
    // can wake p before that.  SMP would need the Rendez's lock held across
    // the sleeper's switch instead.
    q.asleep -= 1;
    q.push(p);
    true
}

/// Sleep on `r` until `cond` holds or `d` has passed, as Plan 9's tsleep:
/// the process's own timer wakes it at the deadline.  A process can't exit
/// while it's here, so the timer is always removed before it's reaped.
pub fn tsleep<A: ProcArch>(r: &Rendez<A>, cond: impl Fn() -> bool, d: Duration) {
    let up = A::up();
    assert!(!up.is_null(), "tsleep: no current process");
    // SAFETY: up is the running process, set by schedule, which resumes it
    // here.  trend and timer are the process's own: while trend is set, the
    // process is asleep, and only its own timer's twakeup touches them.
    unsafe {
        (*up).trend = r;
        (*up).timer = Timer::with_arg(Mode::Relative(d), twakeup::<A>, up as *mut ());
        // SAFETY: the process can't exit before the timer's removed, so the
        // timer is removed before up is freed.
        timer::add_unowned(&(*up).timer);
        // SAFETY: as above.
        sleep(r, || (*up).trend.is_null() || cond());
        timer::remove(&(*up).timer);
    }
}

/// Wake the process the timer `arg` is set to wake, as Plan 9's twakeup:
/// clear its trend, so its sleep's condition holds, then wake it.  Runs in
/// interrupt context, when the timer fires.
pub(crate) fn twakeup<A: ProcArch>(arg: *mut ()) {
    // SAFETY: arg is the process its own tsleep set, and while its trend is
    // set it's asleep, so it's alive, and only its own timer reads and
    // clears its trend.
    unsafe {
        let up = arg as *mut Proc<A>;
        let trend = (*up).trend;
        (*up).trend = ptr::null();
        if !trend.is_null() {
            wakeup(&*trend);
        }
    }
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
    use std::cell::Cell;

    fn start() {}

    /// A 1 kHz clock for tsleep's timeout, recording whether it's armed.
    static CLOCK: crate::clock::Ops =
        crate::clock::Ops { now: || 0, frequency: || 1000, set_timer: |when| ARMED.set(when) };
    thread_local! {
        static ARMED: Cell<Option<u64>> = const { Cell::new(None) };
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
        assert_eq!(unsafe { reap(*p) }, b"bye");
        // SAFETY: as above.
        unsafe { reap(*q) };
        assert_eq!(held(), (0, 0));
    }

    /// Take the next ready process.
    fn next() -> Option<Box<Proc<Fake>>> {
        let node = LockNode::new();
        Fake::run_queue().ilock(&node).pop()
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
    fn sleep_with_its_condition_holding_doesnt_switch() {
        static R: Rendez<Fake> = Rendez::new();
        let mut p = newproc::<Fake>(start).unwrap();
        Fake::set_up(&mut *p);
        // The fake can't switch, so sleeping would panic.
        sleep(&R, || true);
        Fake::set_up(ptr::null_mut());
        // SAFETY: nothing runs in the fake process.
        unsafe { reap(*p) };
    }

    #[test]
    fn wakeup_with_no_sleeper_wakes_nothing() {
        static R: Rendez<Fake> = Rendez::new();
        assert!(!wakeup(&R));
    }

    #[test]
    fn tsleep_whose_condition_holds_leaves_no_timer_queued() {
        crate::clock::init(&CLOCK);
        static R: Rendez<Fake> = Rendez::new();
        let mut p = newproc::<Fake>(start).unwrap();
        Fake::set_up(&mut *p);
        // The fake can't switch, so the condition must already hold: tsleep
        // returns without sleeping, and leaves its timeout unqueued.
        tsleep(&R, || true, Duration::from_millis(10));
        assert_eq!(ARMED.with(|a| a.get()), None, "tsleep's timer is still queued");
        Fake::set_up(ptr::null_mut());
        // SAFETY: nothing runs in the fake process.
        unsafe { reap(*p) };
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
