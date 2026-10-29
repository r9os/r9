//! Channels: the open files a process's descriptors refer to, after Plan 9's
//! chan.c.  A channel is shared by every descriptor that refers to it, as
//! Plan 9's Chan is shared by every fd that names it, so a dup'd descriptor
//! shares its offset, and dropping the last reference closes the file.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::dev::Dev;
use crate::error::{EBADARG, EBADFD, EBADUSEFD, ENOFD};

/// The number of file descriptors a process has.  Plan 9's table grows, as
/// its Fgrp, by DELTAFD at a time, because rfork shares one table between
/// processes; r9 has no rfork and no sharing, so a fixed table the size its
/// programs need is all that's needed.
pub const NOPEN: usize = 16;

/// The open modes, as Plan 9's lib.h: an access, and the flags or'ed in.
pub const OREAD: u32 = 0;
pub const OWRITE: u32 = 1;
pub const ORDWR: u32 = 2;
pub const OEXEC: u32 = 3;
pub const OTRUNC: u32 = 16;
pub const OCEXEC: u32 = 32;
pub const ORCLOSE: u32 = 64;

/// The kinds of file a qid may name, as the type bits of Plan 9's lib.h.
pub const QTDIR: u8 = 0x80;
pub const QTFILE: u8 = 0x00;

/// A file's identity within its device, as in 9P's qid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Qid {
    /// The file's path within the device.
    pub path: u64,
    /// Its kind, a bit of the Qid types.
    pub kind: u8,
}

/// An open file, shared by the descriptors that refer to it, as Plan 9's
/// Chan.  The offset is an atomic, not a plain word: dup'd descriptors share
/// the channel, and its offset is the one thing they must agree on.  It's
/// accessed relaxed, since only the owning process's own syscalls write it,
/// one at a time.
pub struct Chan {
    dev: &'static dyn Dev,
    qid: Qid,
    /// The access the file was opened with: the result of openmode, 0 to 3.
    mode: u32,
    /// The file's length, set by the device on open.
    length: u64,
    offset: AtomicU64,
}

impl Chan {
    /// A new channel on `dev`'s file `qid`, unopened: its mode and length are
    /// 0, and its offset at the start, until the device's open returns the
    /// opened channel.
    pub(crate) fn new(dev: &'static dyn Dev, qid: Qid) -> Arc<Self> {
        Arc::new(Chan { dev, qid, mode: 0, length: 0, offset: AtomicU64::new(0) })
    }

    /// The opened channel on `dev`'s file `qid`: the device's open has
    /// recorded the access, and the file's length, on it, as Plan 9's
    /// devopen does on the channel it opens.
    pub(crate) fn opened(dev: &'static dyn Dev, qid: Qid, mode: u32, length: u64) -> Arc<Self> {
        Arc::new(Chan { dev, qid, mode, length, offset: AtomicU64::new(0) })
    }

    /// The file's device.
    pub(crate) fn dev(&self) -> &'static dyn Dev {
        self.dev
    }

    /// The file's identity within its device.
    pub(crate) fn qid(&self) -> &Qid {
        &self.qid
    }

    /// The access the file was opened with.
    pub(crate) fn mode(&self) -> u32 {
        self.mode
    }

    /// The file's length, as the device set it on open.
    pub(crate) fn length(&self) -> u64 {
        self.length
    }

    pub(crate) fn offset(&self) -> u64 {
        self.offset.load(Ordering::Relaxed)
    }

    pub(crate) fn set_offset(&self, off: u64) {
        self.offset.store(off, Ordering::Relaxed)
    }
}

impl Drop for Chan {
    /// Call the device's close, as Plan 9's cclose does when a channel's
    /// reference count reaches zero.
    fn drop(&mut self) {
        self.dev.close(self);
    }
}

/// Reduce an open mode to the access it names, as Plan 9's openmode: strip
/// the flags, refuse anything that isn't an access, and fold OEXEC into
/// OREAD.
pub(crate) fn openmode(o: u32) -> crate::Result<u32> {
    let o = o & !(OTRUNC | OCEXEC | ORCLOSE);
    if o > OEXEC {
        return Err(EBADARG);
    }
    Ok(if o == OEXEC { OREAD } else { o })
}

/// The channel at descriptor `fd` for an access of `want`, as Plan 9's
/// fdtochan, which takes a -1 mode that accepts any channel and an access
/// that doesn't, and OREAD is 0: r9's unsigned arguments can't carry the -1,
/// so `want` is an Option, None for the -1 that dup, close and seek pass.
/// An out-of-range fd or an empty slot is EBADFD; otherwise the channel must
/// be open ORDWR or exactly `want`, and OTRUNC asked of a read-only channel
/// is EBADUSEFD, as Plan 9's check is.
pub(crate) fn fdtochan(fds: &[Option<Arc<Chan>>], fd: u64, want: Option<u32>) -> crate::Result<Arc<Chan>> {
    let c = fds.get(fd as usize).and_then(Option::as_ref).ok_or(EBADFD)?;
    let want = match want {
        Some(want) if c.mode() != ORDWR => want,
        _ => return Ok(c.clone()),
    };
    if want & OTRUNC != 0 && c.mode() == OREAD {
        return Err(EBADUSEFD);
    }
    if want & !OTRUNC != c.mode() {
        return Err(EBADUSEFD);
    }
    Ok(c.clone())
}

/// The lowest empty descriptor of a process's table, as Plan 9's newfd, or
/// ENOFD if the table is full.
pub(crate) fn freefd(fds: &mut [Option<Arc<Chan>>; NOPEN]) -> crate::Result<usize> {
    for (i, slot) in fds.iter_mut().enumerate() {
        if slot.is_none() {
            return Ok(i);
        }
    }
    Err(ENOFD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ENOTSUP;

    /// A device for the descriptor helpers' tests: nothing on it succeeds.
    struct NoneDev;

    impl Dev for NoneDev {
        fn dc(&self) -> char {
            'n'
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
        fn read(&self, _c: &Arc<Chan>, _buf: &mut [u8], _offset: u64) -> crate::Result<u64> {
            Err(ENOTSUP)
        }
        fn write(&self, _c: &Arc<Chan>, _buf: &[u8], _offset: u64) -> crate::Result<u64> {
            Err(ENOTSUP)
        }
        fn close(&self, _c: &Chan) {}
    }

    /// A channel on the none device's file 1, opened for the access `mode`.
    fn chan(mode: u32) -> Arc<Chan> {
        Chan::opened(&NoneDev, Qid { path: 1, kind: QTFILE }, mode, 0)
    }

    /// An empty table, with `chans` in its first slots.
    fn table(chans: &[Option<Arc<Chan>>]) -> [Option<Arc<Chan>>; NOPEN] {
        let mut fds = [const { None }; NOPEN];
        for (slot, c) in fds.iter_mut().zip(chans) {
            *slot = c.clone();
        }
        fds
    }

    #[test]
    fn openmodes() {
        assert_eq!(openmode(OREAD), Ok(OREAD));
        assert_eq!(openmode(OWRITE), Ok(OWRITE));
        assert_eq!(openmode(ORDWR), Ok(ORDWR));
        assert_eq!(openmode(OEXEC), Ok(OREAD), "OEXEC folds to OREAD");
        assert_eq!(openmode(OWRITE | OTRUNC | OCEXEC | ORCLOSE), Ok(OWRITE), "the flags strip");
        assert_eq!(openmode(4), Err(EBADARG));
        assert_eq!(openmode(u32::MAX), Err(EBADARG), "above OEXEC");
    }

    #[test]
    fn fdtochan_checks() {
        let read = chan(OREAD);
        let write = chan(OWRITE);
        let r = || table(&[Some(read.clone()), Some(write.clone())]);

        assert_eq!(fdtochan(&r(), 0, None).map(|c| c.mode()), Ok(OREAD), "None accepts a read");
        assert_eq!(fdtochan(&r(), 1, None).map(|c| c.mode()), Ok(OWRITE), "None accepts a write");
        assert_eq!(fdtochan(&r(), 0, Some(OREAD)).map(|c| c.mode()), Ok(OREAD), "a read for a read");
        assert_eq!(
            fdtochan(&r(), 0, Some(OWRITE)).map(|c| c.mode()),
            Err(EBADUSEFD),
            "a write for a read"
        );
        assert_eq!(
            fdtochan(&r(), 1, Some(OREAD)).map(|c| c.mode()),
            Err(EBADUSEFD),
            "a read for a write"
        );
        assert_eq!(fdtochan(&r(), 1, Some(OWRITE)).map(|c| c.mode()), Ok(OWRITE), "a write for a write");
        assert_eq!(
            fdtochan(&r(), 0, Some(OWRITE | OTRUNC)).map(|c| c.mode()),
            Err(EBADUSEFD),
            "truncating a read"
        );

        // Plan 9's fdtochan, in all the cases that reach this task.
        let c = chan(ORDWR);
        let fds = table(&[Some(c.clone())]);
        assert_eq!(fdtochan(&fds, 0, Some(OREAD)).map(|c| c.mode()), Ok(ORDWR), "an ordwr for a read");
        assert_eq!(fdtochan(&fds, 0, Some(OWRITE)).map(|c| c.mode()), Ok(ORDWR), "an ordwr for a write");
        assert_eq!(fdtochan(&fds, 0, Some(OWRITE | OTRUNC)).map(|c| c.mode()), Ok(ORDWR), "an ordwr, truncating");

        assert_eq!(fdtochan(&r(), 2, None).map(|c| c.mode()), Err(EBADFD), "an empty slot");
        assert_eq!(
            fdtochan(&r(), NOPEN as u64, None).map(|c| c.mode()),
            Err(EBADFD),
            "out of range"
        );
        assert_eq!(
            fdtochan(&r(), u64::MAX, None).map(|c| c.mode()),
            Err(EBADFD),
            "the -1 of an unsigned fd"
        );
    }

    #[test]
    fn freefds() {
        let write = chan(OWRITE);
        let mut fds = table(&[Some(write.clone()), None, Some(write.clone())]);
        assert_eq!(freefd(&mut fds), Ok(1), "the lowest empty");

        let mut full = [const { None }; NOPEN];
        for slot in &mut full {
            *slot = Some(write.clone());
        }
        assert_eq!(freefd(&mut full), Err(ENOFD), "a full table");
    }
}
