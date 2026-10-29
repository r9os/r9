//! Processes, and the scheduler, after Plan 9's port/proc.c.  Every switch
//! goes through the scheduler's context: a process switches to it to yield
//! or exit, and schedule, running there, re-queues or reaps it, then picks the
//! next.  So a process is reaped off its own stack, and re-queued only once
//! its context is saved.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use core::time::Duration;

use crate::chan::{NOPEN, OREAD, OWRITE};
use crate::dat::{MAXWAIT, Proc, ProcArch, Rendez, State, WaitQueue, Waitmsg};
use crate::dev::open_name;
use crate::error::{EINUSE, ENOCHILD};
use crate::irq;
use crate::mcslock::LockNode;
use crate::pagealloc::{PageAllocError, free_physpage};
use crate::timer::{self, Mode, Timer};

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// How many ticks a process runs before it can be preempted: 100 ms, as Plan
/// 9's HZ/10.
const QUANTUM: u64 = timer::HZ / 10;

/// The current process, for code that isn't generic over the architecture,
/// such as a device: Plan 9's up, as far as such code needs it, and sleep
/// and wakeup.  Each architecture that runs processes registers
/// Current::of::<A>() with init_current, as it does its clock with
/// clock::init.
pub struct Current {
    /// The current process's pid, or None if there's no current process.
    pub pid: fn() -> Option<u64>,
    /// sleep, for the architecture.
    pub sleep: fn(&Rendez, &dyn Fn() -> bool),
    /// wakeup, for the architecture.
    pub wakeup: fn(&Rendez) -> bool,
}

impl Current {
    /// `A`'s current process, sleep and wakeup.
    pub const fn of<A: ProcArch>() -> Current {
        Current { pid: up_pid::<A>, sleep: sleep_dyn::<A>, wakeup: wakeup::<A> }
    }
}

fn sleep_dyn<A: ProcArch>(r: &Rendez, cond: &dyn Fn() -> bool) {
    sleep::<A>(r, cond)
}

#[cfg(not(test))]
static CURRENT: AtomicPtr<Current> = AtomicPtr::new(ptr::null_mut());

// Tests run in parallel threads, so each registers its own.
#[cfg(test)]
std::thread_local! {
    static CURRENT: AtomicPtr<Current> = const { AtomicPtr::new(ptr::null_mut()) };
}

/// Register the architecture's current process.
pub fn init_current(current: &'static Current) {
    let current = ptr::from_ref(current).cast_mut();
    core::cfg_select! {
        test => CURRENT.with(|c| c.store(current, Ordering::Release)),
        _ => CURRENT.store(current, Ordering::Release),
    }
}

fn registered() -> Option<&'static Current> {
    let current = core::cfg_select! {
        test => CURRENT.with(|c| c.load(Ordering::Acquire)),
        _ => CURRENT.load(Ordering::Acquire),
    };
    // SAFETY: CURRENT is null, or was set from a &'static Current by
    // init_current.
    unsafe { current.as_ref() }
}

/// The current process's pid, or None if there's none, or the architecture
/// hasn't registered its current process.
pub fn current_pid() -> Option<u64> {
    registered().and_then(|c| (c.pid)())
}

impl Rendez {
    /// Sleep here until `cond` holds, as sleep, through the architecture's
    /// registered Current, for code that isn't generic over it.
    pub fn sleep(&self, cond: impl Fn() -> bool) {
        let current = registered().expect("Rendez::sleep: no current process registered");
        (current.sleep)(self, &cond)
    }

    /// Wake the process asleep here, if there is one, as wakeup, and return
    /// whether there was.  Without a registered Current, nothing can have
    /// slept through it.  It doesn't allocate, so it can be called from an
    /// interrupt handler.
    pub fn wakeup(&self) -> bool {
        registered().is_some_and(|current| (current.wakeup)(self))
    }
}

fn up_pid<A: ProcArch>() -> Option<u64> {
    // SAFETY: up is null, or the running process, which is alive while it
    // runs.
    unsafe { A::up().as_ref() }.map(|p| p.pid)
}

/// Why newproc failed.
#[derive(Debug, PartialEq)]
pub enum NewprocError {
    AddressSpace(PageAllocError),
    KernelStack(PageAllocError),
}

/// Create a process.  When it's first run, it calls `start`, as Plan 9's
/// kproc runs a function in a new process.  `start` loads the process's
/// program and enters user mode, and returns only if that fails.  Before
/// it runs, the process's standard files are open: the console on 0, OREAD,
/// and on 1 and 2, OWRITE, as Plan 9's opencons.
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
    // The standard files, as Plan 9's opencons: the console on 0, OREAD,
    // and on 1 and 2, OWRITE.  They're opened as #c/cons, which names the
    // device whatever's bound on /dev.
    let mut fds = [const { None }; NOPEN];
    for (slot, mode) in fds.iter_mut().zip([OREAD, OWRITE, OWRITE]) {
        *slot = Some(open_name("#c/cons", mode).expect(
            "newproc: opening #c/cons can't fail: attach has nothing to fail \
             it, cons is in the table, and 0660's group bits grant both modes",
        ));
    }
    Ok(Box::new(Proc {
        pid,
        start,
        pmmu,
        kstack,
        label,
        bss: None,
        fds,
        exit_status: Vec::new(),
        errstr: [0; sys::ERRMAX],
        state: State::Ready,
        fp: A::Fp::default(),
        quantum_end: 0,
        next: ptr::null_mut(),
        sleep: Rendez::new(),
        timer: Timer::new(Mode::Relative(Duration::from_millis(1)), |_| {}),
        trend: ptr::null(),
        text: Vec::new(),
        waitq: Arc::new(WaitQueue::new()),
        parent: None,
    }))
}

/// Make `child` a child of `parent`: its exit goes to `parent`'s wait queue,
/// and counts as one of `parent`'s living children until then, as Plan 9's
/// rfork counts nchild.
#[cfg_attr(not(test), expect(dead_code, reason = "rfork will adopt its children"))]
pub(crate) fn adopt<A: ProcArch>(parent: &Proc<A>, child: &mut Proc<A>) {
    let node = LockNode::new();
    parent.waitq.state.ilock(&node).nchild += 1;
    child.parent = Some(parent.waitq.clone());
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
pub fn sleep<A: ProcArch>(r: &Rendez, cond: impl Fn() -> bool) {
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
            *sleeper = up.cast();
            drop(sleeper);
            // SAFETY: as above.
            unsafe { A::swtch(&mut (*up).label, A::sched()) };
        }
    })
}

/// Wake the process asleep on `r`, if there is one, as Plan 9's wakeup, and
/// return whether there was.  It doesn't allocate, so it can be called from
/// an interrupt handler.
pub fn wakeup<A: ProcArch>(r: &Rendez) -> bool {
    let node = LockNode::new();
    let p = core::mem::replace(&mut *r.sleeper.ilock(&node), ptr::null_mut());
    if p.is_null() {
        return false;
    }
    // SAFETY: the Rendez owned p, a Proc<A> from Box::into_raw, as sleep
    // stored it, and has just given it up.
    let mut p = unsafe { Box::from_raw(p.cast::<Proc<A>>()) };
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
pub fn tsleep<A: ProcArch>(r: &Rendez, cond: impl Fn() -> bool, d: Duration) {
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
        sleep::<A>(r, || (*up).trend.is_null() || cond());
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
            wakeup::<A>(&*trend);
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
/// context, which reaps it.  As Plan 9's pexit does, its files are closed,
/// then its parent, if it has one, is told.
pub fn pexit<A: ProcArch>(status: &[u8]) -> ! {
    let up = A::up();
    assert!(!up.is_null(), "pexit: no current process");
    // SAFETY: up is the running process, set by schedule.
    unsafe {
        (*up).fds = [const { None }; NOPEN];
        tell_parent::<A>(&*up, status);
        (*up).exit_status = status.to_vec();
        (*up).state = State::Exited;
        A::swtch(&mut (*up).label, A::sched());
    }
    unreachable!("pexit: an exited process was resumed")
}

/// Tell `p`'s parent that `p` has exited with `status`, as Plan 9's pexit:
/// unless the parent has exited too, it has one living child fewer, and,
/// unless it already holds MAXWAIT, `p`'s exit goes on the front of its wait
/// queue, newest first, and wakes it.  The message is empty for success,
/// else `p`'s text, pid and status, truncated to ERRMAX - 1 bytes.  And
/// `p`'s own children's exits are no longer queued, as `p` won't wait for
/// them.
fn tell_parent<A: ProcArch>(p: &Proc<A>, status: &[u8]) {
    let node = LockNode::new();
    p.waitq.state.ilock(&node).alive = false;
    let Some(parent) = &p.parent else {
        return;
    };
    let mut msg = Vec::new();
    if !status.is_empty() {
        msg.extend_from_slice(&p.text);
        msg.extend_from_slice(alloc::format!(" {}: ", p.pid).as_bytes());
        msg.extend_from_slice(status);
        // As snprint does, cut it to fit ERRMAX with its NUL, but not in the
        // middle of a UTF-8 character: back up over its continuation bytes.
        if msg.len() > sys::ERRMAX - 1 {
            let mut cut = sys::ERRMAX - 1;
            while cut > 0 && msg[cut] & 0xc0 == 0x80 {
                cut -= 1;
            }
            msg.truncate(cut);
        }
    }
    let mut state = parent.state.ilock(&node);
    if !state.alive {
        return;
    }
    state.nchild -= 1;
    if state.exits.len() < MAXWAIT {
        state.exits.push_front(Waitmsg { pid: p.pid, msg });
        drop(state);
        wakeup::<A>(&parent.waitr);
    }
}

/// Wait for one of the current process's children to exit, and return its
/// exit, as Plan 9's pwait: the newest first.  A second waiter is Einuse, as
/// pwait's canqlock, and a process with no living children and no exits
/// queued is Enochild.
pub fn pwait<A: ProcArch>() -> crate::Result<Waitmsg> {
    let up = A::up();
    assert!(!up.is_null(), "pwait: no current process");
    // SAFETY: up is the running process, set by schedule, and its wait queue
    // lives as long as it does.
    let q = unsafe { &*(*up).waitq };
    {
        let node = LockNode::new();
        let mut state = q.state.ilock(&node);
        if state.waiting {
            return Err(EINUSE);
        }
        if state.nchild == 0 && state.exits.is_empty() {
            return Err(ENOCHILD);
        }
        state.waiting = true;
    }
    sleep::<A>(&q.waitr, || {
        let node = LockNode::new();
        !q.state.ilock(&node).exits.is_empty()
    });
    let node = LockNode::new();
    let mut state = q.state.ilock(&node);
    state.waiting = false;
    Ok(state.exits.pop_front().expect("pwait: woken with no exit queued"))
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
    use alloc::sync::Arc;
    use super::*;
    use crate::chan::QTFILE;
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

    #[test]
    fn newproc_opens_the_console_on_the_standard_fds() {
        let p = newproc::<Fake>(start).expect("newproc failed");
        // The console channel on 0, OREAD, and on 1 and 2, OWRITE.
        for (slot, mode) in p.fds[..3].iter().zip([OREAD, OWRITE, OWRITE]) {
            let c = slot.as_ref().expect("a standard fd");
            assert_eq!(c.qid().path, 1, "the channel's qid path");
            assert_eq!(c.qid().kind, QTFILE, "the channel's qid kind");
            assert_eq!(c.mode(), mode, "the channel's mode");
        }
        // The fds are real channels: reap frees the process's references,
        // and the channel kept across it is left with only the caller's Arc.
        let keep = p.fds[0].clone().expect("fd 0");
        assert_eq!(Arc::strong_count(&keep), 2);
        // SAFETY: nothing runs in the fake process.
        unsafe { reap(*p) };
        assert_eq!(Arc::strong_count(&keep), 1);
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
        static R: Rendez = Rendez::new();
        let mut p = newproc::<Fake>(start).unwrap();
        Fake::set_up(&mut *p);
        // The fake can't switch, so sleeping would panic.
        sleep::<Fake>(&R, || true);
        Fake::set_up(ptr::null_mut());
        // SAFETY: nothing runs in the fake process.
        unsafe { reap(*p) };
    }

    #[test]
    fn wakeup_with_no_sleeper_wakes_nothing() {
        static R: Rendez = Rendez::new();
        assert!(!wakeup::<Fake>(&R));
    }

    #[test]
    fn tsleep_whose_condition_holds_leaves_no_timer_queued() {
        crate::clock::init(&CLOCK);
        static R: Rendez = Rendez::new();
        let mut p = newproc::<Fake>(start).unwrap();
        Fake::set_up(&mut *p);
        // The fake can't switch, so the condition must already hold: tsleep
        // returns without sleeping, and leaves its timeout unqueued.
        tsleep::<Fake>(&R, || true, Duration::from_millis(10));
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

    /// A parent and a child of it, with the child's text set, as exec sets
    /// it.
    fn family() -> (Box<Proc<Fake>>, Box<Proc<Fake>>) {
        let parent = newproc::<Fake>(start).expect("newproc failed");
        let mut child = newproc::<Fake>(start).expect("newproc failed");
        adopt(&parent, &mut child);
        child.text = b"factor".to_vec();
        (parent, child)
    }

    fn state<A: ProcArch>(p: &Proc<A>) -> (Vec<Waitmsg>, usize) {
        let node = LockNode::new();
        let s = p.waitq.state.ilock(&node);
        (s.exits.iter().cloned().collect(), s.nchild)
    }

    /// Reap the fake processes.
    fn reap_all(ps: impl IntoIterator<Item = Box<Proc<Fake>>>) {
        for p in ps {
            // SAFETY: nothing runs in the fake processes.
            unsafe { reap(*p) };
        }
    }

    #[test]
    fn a_childs_exit_reaches_its_parent() {
        let (parent, child) = family();
        let (_, n) = state(&parent);
        assert_eq!(n, 1, "a living child");
        tell_parent(&child, b"no factors");
        let msg = alloc::format!("factor {}: no factors", child.pid).into_bytes();
        assert_eq!(state(&parent), (vec![Waitmsg { pid: child.pid, msg }], 0));

        // Success is an empty message, and the newest exit is first.
        let mut second = newproc::<Fake>(start).expect("newproc failed");
        adopt(&parent, &mut second);
        tell_parent(&second, b"");
        let (exits, _) = state(&parent);
        assert_eq!(exits[0], Waitmsg { pid: second.pid, msg: Vec::new() }, "newest first");
        reap_all([parent, child, second]);
    }

    #[test]
    fn a_long_message_is_truncated() {
        let (parent, child) = family();
        tell_parent(&child, &[b'x'; sys::ERRMAX]);
        let (exits, _) = state(&parent);
        assert_eq!(exits[0].msg.len(), sys::ERRMAX - 1);
        reap_all([parent, child]);
    }

    #[test]
    fn a_message_isnt_cut_mid_character() {
        let (parent, child) = family();
        // "factor N: " then a's to one byte short of the cut, then a
        // two-byte character across it.
        let prefix = alloc::format!("factor {}: ", child.pid).len();
        let mut status = vec![b'a'; sys::ERRMAX - 2 - prefix];
        status.extend_from_slice("é".as_bytes());
        tell_parent(&child, &status);
        let (exits, _) = state(&parent);
        assert_eq!(exits[0].msg.len(), sys::ERRMAX - 2, "the whole é dropped");
        reap_all([parent, child]);
    }

    #[test]
    fn past_maxwait_an_exit_is_dropped() {
        let (parent, child) = family();
        {
            let node = LockNode::new();
            let mut s = parent.waitq.state.ilock(&node);
            for pid in 0..MAXWAIT as u64 {
                s.exits.push_back(Waitmsg { pid, msg: Vec::new() });
            }
        }
        tell_parent(&child, b"");
        let (exits, n) = state(&parent);
        assert_eq!(exits.len(), MAXWAIT, "not queued");
        assert_eq!(n, 0, "but no longer living");
        reap_all([parent, child]);
    }

    #[test]
    fn a_parent_that_has_exited_hears_nothing() {
        let (parent, child) = family();
        tell_parent(&parent, b"");
        tell_parent(&child, b"late");
        assert_eq!(state(&parent), (vec![], 1), "nothing queued, and nchild as it was");
        reap_all([parent, child]);
    }

    #[test]
    fn pwait_takes_the_newest_exit() {
        let (mut parent, child) = family();
        tell_parent(&child, b"");
        Fake::set_up(&mut *parent);
        // The fake can't switch, so an exit must already be queued.
        assert_eq!(pwait::<Fake>(), Ok(Waitmsg { pid: child.pid, msg: Vec::new() }));
        assert_eq!(pwait::<Fake>(), Err(ENOCHILD), "no living children, nothing queued");
        Fake::set_up(ptr::null_mut());
        reap_all([parent, child]);
    }

    #[test]
    fn a_second_waiter_is_einuse() {
        let (mut parent, child) = family();
        {
            let node = LockNode::new();
            parent.waitq.state.ilock(&node).waiting = true;
        }
        Fake::set_up(&mut *parent);
        assert_eq!(pwait::<Fake>(), Err(EINUSE));
        Fake::set_up(ptr::null_mut());
        reap_all([parent, child]);
    }
}
