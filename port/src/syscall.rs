//! System calls, with Plan 9's numbers (libc/9syscall/sys.h).  Each
//! architecture's trap handler takes the number and arguments from its
//! registers, and calls syscall.

use alloc::vec::Vec;

use crate::dat::{Access, ProcArch};
use crate::devcons::putstrn;
use crate::error::EBADFD;
use crate::println;
use crate::proc::pexit;
use crate::segment::ibrk;
use crate::uaccess::{copy_from_user, copy_str_from_user, validate_user_range};

use sys::{BRK_, ERRMAX, EXITS, PWRITE, SYSR1};

/// Run syscall `nr` with `args`, and return its result, or -1 if it failed.
/// The error string is dropped until there's errstr.  An unknown number kills
/// the current process with "sys: bad sys call", which is the effect of Plan
/// 9's note.
pub fn syscall<A: ProcArch>(nr: u64, args: [u64; 6]) -> u64 {
    let r = match nr {
        SYSR1 => sysr1(&args),
        EXITS => sysexits::<A>(&args),
        BRK_ => sysbrk_::<A>(&args),
        PWRITE => syspwrite::<A>(&args),
        _ => {
            println!("bad sys call number {nr}");
            pexit::<A>(b"sys: bad sys call")
        }
    };
    r.unwrap_or(u64::MAX)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, cons_output, fake_cons, user_mem};
    use crate::mem::PAGE_SIZE_4K;

    #[test]
    fn sysr1_returns_0() {
        assert_eq!(syscall::<Fake>(SYSR1, [41, 0, 0, 0, 0, 0]), 0);
    }

    #[test]
    fn pwrite_to_the_console() {
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
