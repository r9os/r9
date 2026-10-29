//! System calls, with Plan 9's numbers (libc/9syscall/sys.h).  Each
//! architecture's trap handler takes the number and arguments from its
//! registers, and calls syscall.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use crate::chan::{fdtochan, freefd, openmode, Chan, NOPEN, OREAD, OWRITE, QTDIR};
use crate::dat::{Access, ProcArch};
use crate::dev::open_name;
use crate::error::{EBADARG, EBADFD, EISDIR, ENAMETOOLONG, ENEGOFF};
use crate::println;
use crate::proc::{pexit, tsleep, yield_now};
use crate::segment::ibrk;
use crate::timer;
use crate::uaccess::{copy_from_user, copy_str_from_user, copy_to_user, peek_user_byte, validate_user_range};

use sys::{BRK_, CLOSE, DUP, ERRMAX, ERRSTR, EXITS, OPEN, PREAD, PWRITE, SEEK, SLEEP, SYSR1};

/// One tick, as Plan 9's TK2MS(1): the shortest SLEEP can sleep.
const TICK: Duration = Duration::from_nanos(1_000_000_000 / timer::HZ);

/// Run syscall `nr` with `args`, and return its result, or -1 if it failed,
/// with its error string recorded in the current process for errstr, as Plan
/// 9's syscall does.  An unknown number kills the current process with "sys:
/// bad sys call", which is the effect of Plan 9's note.
pub fn syscall<A: ProcArch>(nr: u64, args: [u64; 6]) -> u64 {
    let r = match nr {
        SYSR1 => sysr1(&args),
        CLOSE => sysclose::<A>(&args),
        DUP => sysdup::<A>(&args),
        EXITS => sysexits::<A>(&args),
        OPEN => sysopen::<A>(&args),
        BRK_ => sysbrk_::<A>(&args),
        SEEK => sysseek::<A>(&args),
        ERRSTR => syserrstr::<A>(&args),
        PREAD => syspread::<A>(&args),
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

/// Run `f` on the current process's file descriptors.
fn with_fds<A: ProcArch, R>(f: impl FnOnce(&mut [Option<Arc<Chan>>; NOPEN]) -> R) -> R {
    let up = A::up();
    assert!(!up.is_null(), "syscall: no current process");
    // SAFETY: up is the running process, and only its own syscalls, one at a
    // time, use its descriptors.
    f(unsafe { &mut (*up).fds })
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

/// The kernel chunk a pread or pwrite copies the user buffer through, as the
/// 256-byte chunk of Plan 9's conswrite: the device sees only kernel memory,
/// so no user access happens while the console is locked.  A pread is one
/// device read of at most a chunk, as Plan 9's read is one device read: a
/// device that waits, such as cons for a line, mustn't be read again in the
/// same call.
const CHUNK: usize = 256;

/// The offset a pread or pwrite uses, and whether it's the channel's, as
/// Plan 9's offp: the -1 sentinel, u64::MAX, resolves to the channel's
/// offset; any other value is the user's.  Offsets are signed, as Plan 9's
/// vlong, so either one past i64::MAX is Enegoff, and a user range's length
/// added to the result can't overflow u64.
fn offset(c: &Chan, off: u64) -> crate::Result<(u64, bool)> {
    let (off, maintain) = if off == u64::MAX { (c.offset(), true) } else { (off, false) };
    if (off as i64) < 0 {
        return Err(ENEGOFF);
    }
    Ok((off, maintain))
}

/// open(name, mode): the mode checked, as openmode, the name copied out of
/// user memory, the #c/... name resolved to an open channel, and the lowest
/// free descriptor, else Enofd.
fn sysopen<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let (name, mode) = (args[0] as usize, args[1] as u32);
    let omode = openmode(mode)?;
    let mut buf = [0u8; 256];
    let n = copy_str_from_user::<A>(&mut buf, name);
    // A full buffer ends at 256 only if the 257th byte is the terminator or
    // isn't mapped; a mapped byte past it is a longer name, which Plan 9's
    // validname refuses with "name too long" rather than truncating.
    if n == buf.len() && peek_user_byte::<A>(name + n).is_some_and(|b| b != 0) {
        return Err(ENAMETOOLONG);
    }
    let name = core::str::from_utf8(&buf[..n]).map_err(|_| EBADARG)?;
    let c = open_name(name, omode)?;
    with_fds::<A, _>(|fds| {
        let fd = freefd(fds)?;
        fds[fd] = Some(c);
        Ok(fd as u64)
    })
}

/// close(fd): the descriptor's channel, of any mode, taken from the table and
/// dropped, as Plan 9's close.
fn sysclose<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let fd = args[0];
    with_fds::<A, _>(|fds| {
        fdtochan(fds, fd, None)?;
        fds[fd as usize] = None;
        Ok(0)
    })
}

/// dup(old, new): the channel of `old`, of any mode, copied onto `new`, as
/// Plan 9's sysdup.  A `new` of -1 is the lowest free descriptor, else Enofd;
/// any other must be in range, and whatever it held is dropped after the
/// copy, so a dup over an open fd closes the old occupant.
fn sysdup<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let (old, new) = (args[0], args[1]);
    with_fds::<A, _>(|fds| {
        let c = fdtochan(fds, old, None)?;
        let new = if new == u64::MAX { freefd(fds)? } else { new as usize };
        let slot = fds.get_mut(new).ok_or(EBADFD)?;
        let oc = slot.replace(c);
        drop(oc);
        Ok(new as u64)
    })
}

/// pread(fd, buf, len, offset): the buffer checked first, as Plan 9's read,
/// the channel for OREAD, the offset resolved, and one device read, of at
/// most a chunk, copied out.  On a successful read, the channel's offset
/// advances by the bytes read, at an explicit offset too, as Plan 9's read
/// does, though its read(2) and its write don't.
fn syspread<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let (fd, buf, len) = (args[0], args[1] as usize, args[2] as usize);
    validate_user_range::<A>(buf, len, Access::Write);
    let c = with_fds::<A, _>(|fds| fdtochan(fds, fd, Some(OREAD)))?;
    let (off, _) = offset(&c, args[3])?;
    let mut chunk = [0u8; CHUNK];
    let n = CHUNK.min(len);
    let m = c.dev().read(&c, &mut chunk[..n], off)? as usize;
    copy_to_user::<A>(buf, &chunk[..m]);
    // At an explicit offset, the channel's offset wasn't checked, so the add
    // wraps, as Plan 9's vlong does, and a later use of it is Enegoff.
    c.set_offset(c.offset().wrapping_add(m as u64));
    Ok(m as u64)
}

/// pwrite(fd, buf, len, offset): the buffer checked first, as Plan 9's write,
/// the channel for OWRITE, a directory refused, the offset resolved, and the
/// bytes copied through the kernel chunk.  At the -1 offset, the channel's
/// offset advances by the bytes requested before the write, and a short write
/// or a device error rewinds it, as Plan 9's write does.
fn syspwrite<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let (fd, buf, len) = (args[0], args[1] as usize, args[2] as usize);
    validate_user_range::<A>(buf, len, Access::Read);
    let c = with_fds::<A, _>(|fds| fdtochan(fds, fd, Some(OWRITE)))?;
    if c.qid().kind == QTDIR {
        return Err(EISDIR);
    }
    let (off, maintain) = offset(&c, args[3])?;
    if maintain {
        c.set_offset(off + len as u64);
    }
    let mut total = 0;
    let mut err = None;
    let mut chunk = [0u8; CHUNK];
    for pos in (0..len).step_by(CHUNK) {
        let n = CHUNK.min(len - pos);
        copy_from_user::<A>(&mut chunk[..n], buf + pos);
        match c.dev().write(&c, &chunk[..n], off + pos as u64) {
            Ok(m) => {
                total += m as usize;
                // A short write is where the device stopped, as one call of
                // Plan 9's write.
                if (m as usize) < n {
                    break;
                }
            }
            Err(e) => {
                err = Some(e);
                break;
            }
        }
    }
    if maintain {
        // A device error rewinds the whole advance, a short write the
        // difference: the offset is where the write left it.
        match err {
            Some(e) => {
                c.set_offset(off);
                return Err(e);
            }
            None if total < len => c.set_offset(off + total as u64),
            None => {}
        }
    } else if let Some(e) = err {
        return Err(e);
    }
    Ok(total as u64)
}

/// seek(fd, off, whence): the channel of any mode, the base 0, the current
/// offset, or the file's length, for whence 0, 1 or 2, plus the offset, as
/// Plan 9's sseek, and the result stored on the channel and returned.
/// Offsets are signed, as Plan 9's vlong, so a result below 0 or past
/// i64::MAX is Enegoff, and the result can't be the -1 of a failed syscall.
/// A bad whence is Ebadarg.
fn sysseek<A: ProcArch>(args: &[u64; 6]) -> crate::Result<u64> {
    let (fd, off, whence) = (args[0], args[1], args[2]);
    let c = with_fds::<A, _>(|fds| fdtochan(fds, fd, None))?;
    let base = match whence {
        0 => 0,
        1 => c.offset() as i64,
        2 => c.length() as i64,
        _ => return Err(EBADARG),
    };
    let r = base.checked_add(off as i64).filter(|r| *r >= 0).ok_or(ENEGOFF)? as u64;
    c.set_offset(r);
    Ok(r)
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
        tsleep::<A>(r, || false, Duration::from_millis(n as u64).max(TICK));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use super::*;
    use crate::dat::ProcArch;
    use crate::chan::ORDWR;
    use crate::error::{ENONEXIST, ENOFD, EBADUSEFD, ENOTSUP};
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
    fn a_duped_fd_shares_its_offset() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.base();
            fake_cons();
            let pwrite = |fd, len| syscall::<Fake>(PWRITE, [fd, base as u64, len, u64::MAX, 0, 0]);

            mem.0[..6].copy_from_slice(b"hello\n");
            assert_eq!(pwrite(1, 6), 6);
            assert_eq!(syscall::<Fake>(DUP, [1, 3, 0, 0, 0, 0]), 3, "a dup onto a closed fd");
            assert_eq!(pwrite(3, 6), 6);
            // The write through fd 3 left fd 1's offset at 12: the dup shares
            // its channel, and the channel's offset is where both writes left
            // it.
            assert_eq!(syscall::<Fake>(SEEK, [1, 0, 1, 0, 0, 0]), 12);
            assert_eq!(cons_output(), b"hello\r\nhello\r\n");
        });
    }

    #[test]
    fn a_dup_over_an_open_fd_closes_the_old_occupant() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.base();
            fake_cons();
            mem.0[..8].copy_from_slice(b"#c/cons\0");
            let fd = syscall::<Fake>(OPEN, [base as u64, OWRITE as u64, 0, 0, 0, 0]);
            assert_eq!(fd, 3, "the first free fd");
            // The channel's two references: fd 3's slot, and the one kept
            // here to watch its count.
            let keep = {
                // SAFETY: as_proc set up the current process.
                let c = unsafe { (*Fake::up()).fds[3].clone() }.expect("fd 3");
                assert_eq!(Arc::strong_count(&c), 2, "the slot and the keep");
                c
            };
            // A dup of fd 1 onto fd 3 copies fd 1's channel in, and drops
            // whatever fd 3 held.
            assert_eq!(syscall::<Fake>(DUP, [1, 3, 0, 0, 0, 0]), 3, "a dup over an open fd");
            assert_eq!(Arc::strong_count(&keep), 1, "the old occupant's");
            // fd 3 is fd 1's channel now, and a write through it moves the
            // shared offset.
            mem.0[8..14].copy_from_slice(b"hello\n");
            assert_eq!(syscall::<Fake>(PWRITE, [3, (base + 8) as u64, 6, u64::MAX, 0, 0]), 6);
            assert_eq!(syscall::<Fake>(SEEK, [1, 0, 1, 0, 0, 0]), 6, "the shared offset");
            assert_eq!(cons_output(), b"hello\r\n");
        });
    }

    #[test]
    fn an_over_long_name_is_refused() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.base();
            let open = || syscall::<Fake>(OPEN, [base as u64, OWRITE as u64, 0, 0, 0, 0]);

            // "#c/" plus 253 a's: 256 bytes, the buffer's length, then the
            // terminator.  It copies whole and resolves to the walk, whose
            // own error the open reports.
            mem.0[..3].copy_from_slice(b"#c/");
            mem.0[3..256].fill(b'a');
            mem.0[256] = 0;
            assert_eq!(open(), u64::MAX, "256 bytes, the buffer's length");
            assert_eq!(current_errstr(), ENONEXIST.as_bytes());

            // One byte longer, and no terminator: Plan 9's "name too long",
            // not a truncated open.
            mem.0[256] = b'a';
            assert_eq!(open(), u64::MAX, "257 bytes");
            assert_eq!(current_errstr(), ENAMETOOLONG.as_bytes());
        });
    }

    #[test]
    fn seek_from_its_three_whences() {
        as_proc(|| {
            let _mem = user_mem();
            let seek = |fd, off, whence| syscall::<Fake>(SEEK, [fd, off, whence, 0, 0, 0]);

            // The three bases: 0, the current offset, and the file's length,
            // which cons reports as 0.
            assert_eq!(seek(1, 5, 0), 5, "from 0");
            assert_eq!(seek(1, 2, 1), 7, "from the offset");
            assert_eq!(seek(1, 0, 2), 0, "from the length");

            // A closed fd, a fourth whence, and a result below 0, from each
            // base.
            assert_eq!(seek(3, 0, 0), u64::MAX, "a closed fd");
            assert_eq!(current_errstr(), EBADFD.as_bytes());
            assert_eq!(seek(1, 0, 3), u64::MAX, "a fourth whence");
            assert_eq!(current_errstr(), EBADARG.as_bytes());
            assert_eq!(seek(1, u64::MAX, 0), u64::MAX, "below 0, from 0");
            assert_eq!(current_errstr(), ENEGOFF.as_bytes());
            assert_eq!(seek(1, u64::MAX, 1), u64::MAX, "below 0, from the offset");
            assert_eq!(current_errstr(), ENEGOFF.as_bytes());

            // Past i64::MAX, which Plan 9's vlong can't hold, and the
            // offset where the last good seek left it.
            assert_eq!(seek(1, i64::MAX as u64, 0), i64::MAX as u64, "to i64::MAX");
            assert_eq!(seek(1, 1, 1), u64::MAX, "past i64::MAX");
            assert_eq!(current_errstr(), ENEGOFF.as_bytes());
            assert_eq!(seek(1, 0, 1), i64::MAX as u64, "an unchanged offset");
        });
    }

    #[test]
    fn an_offset_past_i64_max_is_negative() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.base();
            fake_cons();
            let seek = |fd, off, whence| syscall::<Fake>(SEEK, [fd, off, whence, 0, 0, 0]);
            let pwrite = |off| syscall::<Fake>(PWRITE, [1, base as u64, 6, off, 0, 0]);
            mem.0[..6].copy_from_slice(b"hello\n");

            // A -1-offset write at i64::MAX succeeds, and leaves the channel's
            // offset past it, as Plan 9's vlong would go negative.
            assert_eq!(seek(1, i64::MAX as u64, 0), i64::MAX as u64);
            assert_eq!(pwrite(u64::MAX), 6, "a write at i64::MAX");

            // From there, every use of the offset is Enegoff.
            assert_eq!(pwrite(u64::MAX), u64::MAX, "a write past i64::MAX");
            assert_eq!(current_errstr(), ENEGOFF.as_bytes());
            assert_eq!(seek(1, 0, 1), u64::MAX, "a seek from past i64::MAX");
            assert_eq!(current_errstr(), ENEGOFF.as_bytes());

            // An explicit offset is unaffected, and a seek from 0 recovers.
            assert_eq!(pwrite(0), 6, "an explicit offset");
            assert_eq!(seek(1, 0, 0), 0, "a seek from 0");
            assert_eq!(cons_output(), b"hello\r\nhello\r\n");
        });
    }

    /// A device whose every read and write transfers at most SHORT bytes, as
    /// a console line or a file's end would.
    struct ShortDev;

    const SHORT: usize = 5;

    impl crate::dev::Dev for ShortDev {
        fn dc(&self) -> char {
            's'
        }
        fn attach(&self) -> crate::Result<Arc<Chan>> {
            Err(ENOTSUP)
        }
        fn walk(&self, _c: &Arc<Chan>, _elems: &[&str]) -> crate::Result<Arc<Chan>> {
            Err(ENOTSUP)
        }
        fn open(&self, _c: &Arc<Chan>, _omode: u32) -> crate::Result<Arc<Chan>> {
            Err(ENOTSUP)
        }
        fn read(&self, _c: &Arc<Chan>, buf: &mut [u8], _offset: u64) -> crate::Result<u64> {
            let n = buf.len().min(SHORT);
            buf[..n].fill(b's');
            Ok(n as u64)
        }
        fn write(&self, _c: &Arc<Chan>, buf: &[u8], _offset: u64) -> crate::Result<u64> {
            Ok(buf.len().min(SHORT) as u64)
        }
        fn close(&self, _c: &Chan) {}
    }

    #[test]
    fn a_short_transfer_ends_the_call() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.base();
            let qid = crate::chan::Qid { path: 0, kind: crate::chan::QTFILE };
            // SAFETY: as_proc set up the current process, and its syscalls
            // run one at a time.
            unsafe { (*Fake::up()).fds[3] = Some(Chan::opened(&ShortDev, qid, ORDWR, 0)) };
            let seek = |off, whence| syscall::<Fake>(SEEK, [3, off, whence, 0, 0, 0]);

            // A read longer than a chunk gets what the device's one short
            // read gave, and nothing after it is touched.
            mem.0[..600].fill(0);
            assert_eq!(syscall::<Fake>(PREAD, [3, base as u64, 600, u64::MAX, 0, 0]), 5);
            assert_eq!(&mem.0[..5], b"sssss");
            assert!(mem.0[5..600].iter().all(|&b| b == 0), "the rest of the buffer");
            assert_eq!(seek(0, 1), 5, "the offset, by the bytes read");

            // A write the same: the offset is where the short write left it.
            assert_eq!(syscall::<Fake>(PWRITE, [3, base as u64, 600, u64::MAX, 0, 0]), 5);
            assert_eq!(seek(0, 1), 10, "the offset, by the bytes written");
        });
    }

    #[test]
    fn a_dup_onto_minus_1_takes_the_lowest_free_fd() {
        as_proc(|| {
            let dup = |old, new| syscall::<Fake>(DUP, [old, new, 0, 0, 0, 0]);
            assert_eq!(dup(1, u64::MAX), 3, "the first free fd");
            assert_eq!(dup(1, 5), 5);
            assert_eq!(dup(1, u64::MAX), 4, "below the one just taken");
            for fd in 6..NOPEN as u64 {
                assert_eq!(dup(1, u64::MAX), fd);
            }
            assert_eq!(dup(1, u64::MAX), u64::MAX, "a full table");
            assert_eq!(current_errstr(), ENOFD.as_bytes());
            assert_eq!(dup(1, NOPEN as u64), u64::MAX, "out of range");
            assert_eq!(current_errstr(), EBADFD.as_bytes());
        });
    }

    #[test]
    fn a_pread_of_a_write_only_fd() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.base();
            let pread = |fd, len| syscall::<Fake>(PREAD, [fd, base as u64, len, u64::MAX, 0, 0]);

            // cons is write-only in this task: a read of its fd is an
            // inappropriate use, and the offset stays where the read left it.
            assert_eq!(pread(1, 6), u64::MAX, "a cons read");
            assert_eq!(current_errstr(), EBADUSEFD.as_bytes());
            assert_eq!(syscall::<Fake>(SEEK, [1, 0, 1, 0, 0, 0]), 0, "an unchanged offset");

            // A directory's read isn't supported yet: #c's root, opened
            // OREAD, and its ENOTSUP leaves the offset at the start, too.
            mem.0[..3].copy_from_slice(b"#c\0");
            let fd = syscall::<Fake>(OPEN, [base as u64, OREAD as u64, 0, 0, 0, 0]);
            assert_eq!(fd, 3, "the first free fd");
            assert_eq!(pread(3, 6), u64::MAX, "a directory read");
            assert_eq!(current_errstr(), ENOTSUP.as_bytes());
            assert_eq!(syscall::<Fake>(SEEK, [3, 0, 1, 0, 0, 0]), 0, "an unchanged offset");
        });
    }

    #[test]
    fn reads_of_time_and_pid() {
        as_proc(|| {
            let mut mem = user_mem();
            let base = mem.base();
            static CLOCK: crate::clock::Ops =
                crate::clock::Ops { now: || 1000, frequency: || 1000, set_timer: |_| {} };
            crate::clock::init(&CLOCK);
            static CURRENT: crate::proc::Current = crate::proc::Current::of::<Fake>();
            crate::proc::init_current(&CURRENT);
            let mut open = |name: &[u8]| {
                mem.0[..name.len()].copy_from_slice(name);
                syscall::<Fake>(OPEN, [base as u64, OREAD as u64, 0, 0, 0, 0])
            };
            assert_eq!(open(b"/dev/time\0"), 3);
            assert_eq!(open(b"/dev/pid\0"), 4);
            let pread = |fd, len, off| syscall::<Fake>(PREAD, [fd, base as u64, len, off, 0, 0]);
            let seek = |fd, off, whence| syscall::<Fake>(SEEK, [fd, off, whence, 0, 0, 0]);

            // As cat reads it: all of it, then nothing, its end.
            assert_eq!(pread(3, 600, u64::MAX), 78, "the whole text");
            assert_eq!(pread(3, 600, u64::MAX), 0, "the end");

            // A read at an explicit offset advances the channel's offset by
            // the bytes read, as Plan 9's read does.
            assert_eq!(seek(3, 0, 0), 0);
            assert_eq!(pread(3, 600, 70), 8, "the tail");
            assert_eq!(seek(3, 0, 1), 8, "advanced from 0, not from 70");

            // The reading process's pid.
            // SAFETY: as_proc set up the current process.
            let pid = unsafe { (*Fake::up()).pid };
            assert_eq!(pread(4, 600, 0), 12);
            assert_eq!(&mem.0[..12], format!("{pid:>11} ").as_bytes());
        });
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
