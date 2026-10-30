//! Access to user memory from syscalls, as Plan 9's validaddr in
//! port/fault.c.
//!
//! Each page is checked with MMU::user_page_ok, then the user address is used
//! directly.  That's correct only while nothing can change the mapping between
//! the check and the access: one CPU, no preemption, and one thread per
//! process.
//!
//! A bad address kills the process with "sys: bad address in syscall", which
//! is the effect of Plan 9's validaddr, so these functions don't return on
//! failure.  Callers must hold nothing that needs releasing, such as a KMap.

use crate::dat::{Access, ProcArch};
use crate::mem::PAGE_SIZE_4K;
use crate::proc::pexit;

/// Whether user mode could make `access` to every byte of [addr, addr + len).
/// A range of no bytes is fine if it's within the user address space.
pub(crate) fn user_range_ok<A: ProcArch>(addr: usize, len: usize, access: Access) -> bool {
    let Some(end) = addr.checked_add(len) else {
        return false;
    };
    if end > A::UADDRTOP {
        return false;
    }
    if len == 0 {
        return true;
    }
    (addr & !(PAGE_SIZE_4K - 1)..end).step_by(PAGE_SIZE_4K).all(|va| A::user_page_ok(va, access))
}

/// The byte at user address `addr`, if user mode could read it; None if the
/// byte's page isn't mapped, where a string ends.
pub(crate) fn peek_user_byte<A: ProcArch>(addr: usize) -> Option<u8> {
    if !user_range_ok::<A>(addr, 1, Access::Read) {
        return None;
    }
    // SAFETY: the page is mapped readable, and stays so until the syscall
    // returns.
    Some(unsafe { core::ptr::read(addr as *const u8) })
}

/// Kill the current process unless user mode could make `access` to every
/// byte of [addr, addr + len).
pub fn validate_user_range<A: ProcArch>(addr: usize, len: usize, access: Access) {
    if !user_range_ok::<A>(addr, len, access) {
        bad_address::<A>();
    }
}

/// Kill the current process for a bad address in a syscall, as Plan 9's
/// validaddr does with its note.
pub(crate) fn bad_address<A: ProcArch>() -> ! {
    pexit::<A>(b"sys: bad address in syscall")
}

/// Copy `dst.len()` bytes from user address `src` into `dst`, or kill the
/// current process if they can't all be read.
pub fn copy_from_user<A: ProcArch>(dst: &mut [u8], src: usize) {
    validate_user_range::<A>(src, dst.len(), Access::Read);
    // SAFETY: the whole range is mapped readable, and stays so until the
    // syscall returns.  dst is kernel memory, so they don't overlap.
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, dst.as_mut_ptr(), dst.len()) };
}

/// Copy `src` to user address `dst`, or kill the current process if it can't
/// all be written.
pub fn copy_to_user<A: ProcArch>(dst: usize, src: &[u8]) {
    validate_user_range::<A>(dst, src.len(), Access::Write);
    // SAFETY: the whole range is mapped writable, and stays so until the
    // caller returns.  src is kernel memory, so they don't overlap.
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len()) };
}

/// Copy the NUL-terminated string at user address `src` into `dst`, without
/// its NUL, as Linux's strncpy_from_user.  Return its length, or `dst.len()` if
/// there's no NUL within `dst.len()` bytes, in which case it's truncated.
/// Each page is checked before it's read, so a string may end just before an
/// unmapped page.  Kill the current process if a byte it needs can't be read.
pub fn copy_str_from_user<A: ProcArch>(dst: &mut [u8], src: usize) -> usize {
    try_copy_str_from_user::<A>(dst, src).unwrap_or_else(|| bad_address::<A>())
}

/// As copy_str_from_user, but None for a byte it can't read, for a caller
/// with something to free before the process is killed.
pub(crate) fn try_copy_str_from_user<A: ProcArch>(dst: &mut [u8], src: usize) -> Option<usize> {
    let mut n = 0;
    while n < dst.len() {
        // The first check puts src below UADDRTOP, so this can't overflow.
        let va = src + n;
        if !user_range_ok::<A>(va, 1, Access::Read) {
            return None;
        }
        let page_end = (va & !(PAGE_SIZE_4K - 1)) + PAGE_SIZE_4K;
        let chunk = (page_end - va).min(dst.len() - n);
        for i in 0..chunk {
            // SAFETY: the page containing va..va + chunk is mapped readable.
            let b = unsafe { core::ptr::read((va + i) as *const u8) };
            if b == 0 {
                return Some(n + i);
            }
            dst[n + i] = b;
        }
        n += chunk;
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, UADDRTOP, user_mem};

    #[test]
    fn user_range_ok_ranges() {
        let mem = user_mem();
        let base = mem.base();
        let ok = user_range_ok::<Fake>;
        assert!(ok(base, 2 * PAGE_SIZE_4K, Access::Read));
        assert!(ok(base + 10, PAGE_SIZE_4K, Access::Read), "a range spanning pages");
        assert!(
            !ok(base + PAGE_SIZE_4K - 1, 2, Access::Write),
            "a write reaching a read-only page"
        );
        assert!(
            !ok(base + PAGE_SIZE_4K, 2 * PAGE_SIZE_4K, Access::Read),
            "reaching an unmapped page"
        );
        assert!(ok(base + 2 * PAGE_SIZE_4K + 1, 0, Access::Read), "an empty range");
        assert!(ok(UADDRTOP, 0, Access::Read), "an empty range at the top");
        assert!(!ok(UADDRTOP, 1, Access::Read), "past the top");
        assert!(!ok(UADDRTOP - 1, 2, Access::Read), "crossing the top");
        assert!(!ok(usize::MAX, 2, Access::Read), "overflow");
    }

    #[test]
    fn copy_from_user_copies() {
        let mut mem = user_mem();
        let base = mem.base();
        mem.0[PAGE_SIZE_4K - 2..PAGE_SIZE_4K + 2].copy_from_slice(b"abcd");
        let mut buf = [0; 4];
        copy_from_user::<Fake>(&mut buf, base + PAGE_SIZE_4K - 2);
        assert_eq!(&buf, b"abcd");
    }

    #[test]
    fn copy_to_user_copies() {
        let mut mem = user_mem();
        let base = mem.0.as_mut_ptr() as usize;
        copy_to_user::<Fake>(base + 8, b"abcd");
        copy_to_user::<Fake>(base + PAGE_SIZE_4K - 2, b"ef");
        copy_to_user::<Fake>(base, b"");
        assert_eq!(&mem.0[8..12], b"abcd");
        assert_eq!(&mem.0[PAGE_SIZE_4K - 2..PAGE_SIZE_4K], b"ef", "up to a read-only page");
    }

    #[test]
    fn copy_str_from_user_strings() {
        let mut mem = user_mem();
        let base = mem.base();
        let mut buf = [0; 8];

        mem.0[..4].copy_from_slice(b"bye\0");
        assert_eq!(copy_str_from_user::<Fake>(&mut buf, base), 3);
        assert_eq!(&buf[..3], b"bye");

        let at = PAGE_SIZE_4K - 2;
        mem.0[at..at + 5].copy_from_slice(b"span\0");
        assert_eq!(copy_str_from_user::<Fake>(&mut buf, base + at), 4, "a string spanning pages");
        assert_eq!(&buf[..4], b"span");

        mem.0[..8].copy_from_slice(b"too long");
        assert_eq!(copy_str_from_user::<Fake>(&mut buf, base), 8, "no NUL, so truncated");
        assert_eq!(&buf, b"too long");

        let at = 2 * PAGE_SIZE_4K - 3;
        mem.0[at..at + 3].copy_from_slice(b"ro\0");
        assert_eq!(
            copy_str_from_user::<Fake>(&mut buf, base + at),
            2,
            "ending just before an unmapped page"
        );
        assert_eq!(&buf[..2], b"ro");
    }
}
