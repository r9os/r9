//! System calls, with Plan 9's numbers (libc/9syscall/sys.h).  Each
//! architecture's trap handler takes the number and arguments from its
//! registers, and calls syscall.

use alloc::vec::Vec;
use core::time::Duration;

use crate::dat::{Access, ProcArch};
use crate::devcons::putstrn;
use crate::error::{EBADARG, EBADFD};
use crate::println;
use crate::proc::{pexit, tsleep, yield_now};
use crate::segment::ibrk;
use crate::timer;
use crate::uaccess::{copy_from_user, copy_str_from_user, copy_to_user, validate_user_range};

use sys::{BRK_, ERRMAX, ERRSTR, EXITS, PWRITE, SLEEP, SYSR1};

/// One tick, as Plan 9's TK2MS(1): the shortest SLEEP can sleep.
const TICK: Duration = Duration::from_nanos(1_000_000_000 / timer::HZ);

/// Run syscall `nr` with `args`, and return its result, or -1 if it failed,
/// with its error string recorded in the current process for errstr, as Plan
/// 9's syscall does.  An unknown number kills the current process with "sys:
/// bad sys call", which is the effect of Plan 9's note.
pub fn syscall<A: ProcArch>(nr: u64, args: [u64; 6]) -> u64 {
    let r = match nr {
        SYSR1 => sysr1(&args),
        EXITS => sysexits::<A>(&args),
        BRK_ => sysbrk_::<A>(&args),
        ERRSTR => syserrstr::<A>(&args),
        PWRITE => syspwrite::<A>(&args),
        SLEEP => syssleep::<A>(&args),
        _ => {
            println!("bad sys call number {nr}");
            pexit::<A>(b"sys: bad sys call")
        }
    };
    r.unwrap_or_else(|err| {
        set_errstr::<A>(err);
        u64::MAX
    })
}

/// Run `f` on the current process's error string.
fn with_errstr<A: ProcArch, R>(f: impl FnOnce(&mut [u8; ERRMAX]) -> R) -> R {
    let up = A::up();
    assert!(!up.is_null(), "syscall: no current process");
    // SAFETY: up is the running process, and only its own syscalls, one at a
    // time, use its error string.
    f(unsafe { &mut (*up).errstr })
}

/// Record `err` as the current process's error string, truncated to ERRMAX - 1
/// bytes.
fn set_errstr<A: ProcArch>(err: &str) {
    with_errstr::<A, _>(|errstr| {
        let n = err.len().min(ERRMAX - 1);
        errstr[..n].copy_from_slice(&err.as_bytes()[..n]);
        errstr[n] = 0;
    });
}

/// errstr(buf, nbuf): swap the current process's error string with the
/// NUL-terminated string in `buf`, as Plan 9's generrstr.  Both are truncated
/// to fit, and NUL-terminated, in min(nbuf, ERRMAX) bytes.
fn syserrstr<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let (buf, nbuf) = (args[0] as usize, args[1] as usize);
    if nbuf == 0 {
        return Err(EBADARG);
    }
    validate_user_range::<A>(buf, nbuf, Access::Write);
    let n = nbuf.min(ERRMAX);
    let mut tmp = [0; ERRMAX];
    copy_from_user::<A>(&mut tmp[..n], buf);
    tmp[n - 1] = 0;
    // The whole buffer was checked, so these copies can't kill the process.
    with_errstr::<A, _>(|errstr| {
        copy_to_user::<A>(buf, &errstr[..n - 1]);
        copy_to_user::<A>(buf + n - 1, &[0]);
        errstr[..n].copy_from_slice(&tmp[..n]);
    });
    Ok(0)
}

fn sysr1(_args: &[u64; 6]) -> crate::Result<u64> {
    Ok(0)
}

fn sysexits<A: ProcArch>(args: &[u64; 6]) -> ! {
    pexit::<A>(&exits_status::<A>(args[0] as usize))
}

fn sysbrk_<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let up = A::up();
    assert!(!up.is_null(), "sysbrk_: no current process");
    // SAFETY: up is the running process, and nothing else refers to its bss
    // while it's in a syscall.
    ibrk::<A>(unsafe { &mut (*up).bss }, args[0] as usize)
}

/// pwrite(fd, buf, len, offset).  Only fds 1 and 2 are open, and both are the
/// console, which ignores the offset, as Plan 9's conswrite does.  The whole
/// buffer is checked first, as in Plan 9's write, then copied through a stack
/// buffer, so that no user access happens with the console locked.
fn syspwrite<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let (fd, buf, len) = (args[0], args[1] as usize, args[2] as usize);
    validate_user_range::<A>(buf, len, Access::Read);
    if fd != 1 && fd != 2 {
        return Err(EBADFD);
    }
    let mut chunk = [0; 256];
    for off in (0..len).step_by(chunk.len()) {
        let n = chunk.len().min(len - off);
        copy_from_user::<A>(&mut chunk[..n], buf + off);
        putstrn(&chunk[..n]);
    }
    Ok(len as u64)
}

/// The status for exits(ptr): empty for a null pointer, otherwise the string
/// at `ptr`, truncated to ERRMAX - 1 bytes as Plan 9's sysexits does.
fn exits_status<A: ProcArch>(ptr: usize) -> Vec<u8> {
    if ptr == 0 {
        return Vec::new();
    }
    let mut buf = [0; ERRMAX - 1];
    let n = copy_str_from_user::<A>(&mut buf, ptr);
    buf[..n].to_vec()
}

/// sleep(ms): yield if zero or negative, else sleep at least one tick, as
/// Plan 9's syssleep.  Only its timeout wakes it, and it returns 0.
fn syssleep<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let up = A::up();
    assert!(!up.is_null(), "syssleep: no current process");
    let n = args[0] as i64;
    if n <= 0 {
        yield_now::<A>();
    } else {
        // SAFETY: up is the running process, set by schedule, which resumes
        // it here.  Its sleep Rendez is its own, and its syscalls run one at
        // a time.
        let r = unsafe { &(*up).sleep };
        tsleep(r, || false, Duration::from_millis(n as u64).max(TICK));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::ProcArch;
    use crate::fake::{Fake, cons_output, fake_cons, user_mem};
    use crate::mem::PAGE_SIZE_4K;
    use crate::proc::{newproc, reap};

    /// Run `f` as a new process, which is then reaped.
    fn as_proc<R>(f: impl FnOnce() -> R) -> R {
        let mut p = newproc::<Fake>(|| {}).expect("newproc failed");
        Fake::set_up(&mut *p);
        let r = f();
        Fake::set_up(core::ptr::null_mut());
        // SAFETY: nothing runs in the fake process.
        unsafe { reap(*p) };
        r
    }

    /// The current process's error string, up to its NUL.
    fn current_errstr() -> Vec<u8> {
        // SAFETY: as_proc set up the current process.
        let e = unsafe { (*Fake::up()).errstr };
        e[..e.iter().position(|&b| b == 0).unwrap()].to_vec()
    }

    #[test]
    fn sysr1_returns_0() {
        assert_eq!(syscall::<Fake>(SYSR1, [41, 0, 0, 0, 0, 0]), 0);
    }

    #[test]
    fn pwrite_to_the_console() {
        as_proc(pwrite_to_the_console_as_proc);
    }

    fn pwrite_to_the_console_as_proc() {
        let mut mem = user_mem();
        let base = mem.base();
        fake_cons();
        let pwrite = |fd, buf, len| syscall::<Fake>(PWRITE, [fd, buf as u64, len, u64::MAX, 0, 0]);

        mem.0[..6].copy_from_slice(b"hello\n");
        assert_eq!(pwrite(1, base, 6), 6);
        assert_eq!(pwrite(2, base, 5), 5);
        assert_eq!(cons_output(), b"hello\r\nhello");

        assert_eq!(pwrite(1, base, 0), 0, "empty");
        assert_eq!(pwrite(3, base, 6), u64::MAX, "bad fd");
        assert_eq!(cons_output(), b"");

        // Longer than one chunk, and into the read-only page.
        let at = PAGE_SIZE_4K - 10;
        let text: Vec<u8> = (0..300).map(|i| b'a' + (i % 26) as u8).collect();
        mem.0[at..at + text.len()].copy_from_slice(&text);
        assert_eq!(pwrite(1, base + at, 300), 300);
        assert_eq!(cons_output(), text);
    }

    #[test]
    fn errstr_swaps() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.0.as_mut_ptr() as usize;
            let errstr = |buf: usize, n: u64| syscall::<Fake>(ERRSTR, [buf as u64, n, 0, 0, 0, 0]);
            assert_eq!(current_errstr(), b"", "a new process's");

            // A failed syscall records its error, and a successful one keeps it.
            assert_eq!(syscall::<Fake>(PWRITE, [3, base as u64, 0, u64::MAX, 0, 0]), u64::MAX);
            assert_eq!(syscall::<Fake>(SYSR1, [0; 6]), 0);
            assert_eq!(current_errstr(), EBADFD.as_bytes());

            // errstr swaps it with the buffer's string.
            mem.0[..4].copy_from_slice(b"old\0");
            assert_eq!(errstr(base, ERRMAX as u64), 0);
            let n = EBADFD.len();
            assert_eq!(&mem.0[..=n], [EBADFD.as_bytes(), b"\0"].concat());
            assert_eq!(current_errstr(), b"old");

            // A short buffer truncates both, and NUL-terminates them.
            mem.0[..6].copy_from_slice(b"abcdef");
            assert_eq!(errstr(base, 3), 0);
            assert_eq!(&mem.0[..3], b"ol\0");
            assert_eq!(current_errstr(), b"ab");

            // A zero-length buffer is a bad argument.
            assert_eq!(errstr(base, 0), u64::MAX);
            assert_eq!(current_errstr(), EBADARG.as_bytes());
        });
    }

    #[test]
    fn exits_statuses() {
        let mut mem = user_mem();
        let base = mem.base();
        assert_eq!(exits_status::<Fake>(0), b"", "null");

        mem.0[..4].copy_from_slice(b"bye\0");
        assert_eq!(exits_status::<Fake>(base), b"bye");

        mem.0[..ERRMAX + 1].fill(b'a');
        assert_eq!(exits_status::<Fake>(base), [b'a'; ERRMAX - 1], "truncated");
    }
}
