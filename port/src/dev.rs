//! Devices, and the resolution of a #c/... name to an open channel, as
//! Plan 9's dev.c and the # branch of namec in chan.c.

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::chan::{self, Chan, OREAD};
use crate::devcons;
use crate::error::EBADSHARP;

/// A device's operations for the files it holds, as Plan 9's Device, with
/// the six this kernel needs; Plan 9's other eleven have no caller until the
/// plan reaches them.  The read and write slices are kernel-resident: the
/// copy to and from user memory is the syscall layer's, and it's generic over
/// the architecture, so a device never touches user memory.
pub trait Dev: Sync {
    /// The device's letter, the character after # in its files' names.
    fn dc(&self) -> char;

    /// A fresh channel on the device's root directory, unopened, as Plan 9's
    /// attach.
    fn attach(&self) -> crate::Result<Arc<Chan>>;

    /// Descend the name elements and return a fresh channel on the file they
    /// name, unopened, as Plan 9's namec makes a channel at each step of its
    /// walk.  A walk of no elements is a no-op.
    fn walk(&self, c: &Arc<Chan>, elems: &[&str]) -> crate::Result<Arc<Chan>>;

    /// Open the file for the access `omode`, the result of openmode, and
    /// return the opened channel, as Plan 9's devopen: the permission check,
    /// the offset to zero, a directory OREAD only, and the mode and the
    /// file's length recorded.
    fn open(&self, c: &Arc<Chan>, omode: u32) -> crate::Result<Arc<Chan>>;

    /// Read into `buf` from `offset`, and return the bytes read, as Plan 9's
    /// devread.
    fn read(&self, c: &Arc<Chan>, buf: &mut [u8], offset: u64) -> crate::Result<u64>;

    /// Write `buf` at `offset`, and return the bytes written, as Plan 9's
    /// devwrite.
    fn write(&self, c: &Arc<Chan>, buf: &[u8], offset: u64) -> crate::Result<u64>;

    /// Close the file, as Plan 9's close, when the channel's last reference
    /// is being dropped, from Chan's Drop, as cclose does at a zero count.
    fn close(&self, c: &Chan);
}

/// The devices, as Plan 9's devtab: one static each, looked up by dc, as
/// devno does.
static DEVS: &[&dyn Dev] = &[&devcons::CONS];

/// The device with dc `dc`, as Plan 9's devno scans its table.
fn devno(dc: char) -> Option<&'static dyn Dev> {
    DEVS.iter().find(|d| d.dc() == dc).copied()
}

/// The device and path a name names, as the # branch of Plan 9's namec: the
/// name must be a #, a device's letter, and either nothing, a bare #c, or a
/// / and the path from the device's root; any other name is EBADSHARP, as
/// the device is unknown.
fn devpath(name: &str) -> crate::Result<(&'static dyn Dev, &str)> {
    let after_hash = name.strip_prefix('#').ok_or(EBADSHARP)?;
    let mut chars = after_hash.chars();
    let Some(dc) = chars.next() else {
        return Err(EBADSHARP);
    };
    let path = match after_hash.get(dc.len_utf8()..).ok_or(EBADSHARP)? {
        "" => "",
        p if p.starts_with('/') => &p[1..],
        _ => return Err(EBADSHARP),
    };
    devno(dc).ok_or(EBADSHARP).map(|d| (d, path))
}

/// The elements of a path: the names between its slashes.  An empty path has
/// none, and a doubled or trailing slash is an empty element, which a walk
/// turns into ENONEXIST, as Plan 9's devstat errors.
fn split(path: &str) -> Vec<&str> {
    if path.is_empty() {
        return Vec::new();
    }
    path.split('/').collect()
}

/// The channel `name` opens for the access `omode`, as the # branch of Plan
/// 9's namec: the mode is checked first, as Plan 9's openmode, then the
/// device is attached, the path walked, and the channel opened.
#[cfg_attr(not(test), expect(dead_code, reason = "newproc and the file syscalls use it"))]
pub(crate) fn open_name(name: &str, omode: u32) -> crate::Result<Arc<Chan>> {
    let omode = chan::openmode(omode)?;
    let (dev, path) = devpath(name)?;
    let elems = split(path);
    let c = dev.attach()?;
    let c = dev.walk(&c, &elems)?;
    dev.open(&c, omode)
}

/// The group slot of a file mode, as the bits of it the permission check
/// uses: r9's processes are Plan 9's system user, so there is no owner or
/// other slot to pick.
fn group(mode: u32) -> u32 {
    (mode >> 3) & 7
}

/// The access an open mode asks for, as the bits of Plan 9's access[] in
/// devpermcheck: read, write, both, or execute.
fn access(omode: u32) -> u32 {
    match omode & 3 {
        OREAD => 4,
        1 => 2,
        2 => 6,
        _ => 1,
    }
}

/// Whether the group slot of `mode` grants `omode`, as Plan 9's
/// devpermcheck, which uses the owner, group, or other slot by the caller's
/// identity, but r9 has no user model, so the group slot for everyone.
pub(crate) fn perm_ok(mode: u32, omode: u32) -> bool {
    let want = access(omode);
    (want & group(mode)) == want
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chan::{OWRITE, ORDWR, OTRUNC, QTDIR, QTFILE};
    use crate::error::{EBADARG, ENONEXIST, ENOTSUP, ENOTDIR, EPERM};
    use crate::fake::{cons_output, fake_cons};

    /// The path and device a name resolves to, as devpath does.
    fn path_of(name: &str) -> Result<(char, &str), &'static str> {
        devpath(name).map(|(d, p)| (d.dc(), p))
    }

    /// The error a name's open fails with.
    fn err(name: &str, omode: u32) -> &'static str {
        open_name(name, omode).err().expect("the open to fail")
    }

    #[test]
    fn devpaths() {
        assert_eq!(path_of("#c"), Ok(('c', "")), "bare");
        assert_eq!(path_of("#c/cons"), Ok(('c', "cons")));
        assert_eq!(path_of("#c/cons/"), Ok(('c', "cons/")));
        assert_eq!(path_of("#c//cons"), Ok(('c', "/cons")));
        assert_eq!(path_of("#c/"), Ok(('c', "")), "a trailing slash is a no walk");
        assert_eq!(path_of("#x"), Err(EBADSHARP), "an unknown device");
        assert_eq!(path_of("#ccons"), Err(EBADSHARP), "no slash after the letter");
        assert_eq!(path_of("c"), Err(EBADSHARP), "no #");
        assert_eq!(path_of("#"), Err(EBADSHARP), "no letter");
        assert_eq!(path_of(""), Err(EBADSHARP), "nothing");
    }

    #[test]
    fn splits() {
        assert!(split("").is_empty(), "an empty path has no elements");
        assert_eq!(split("cons"), vec!["cons"]);
        assert_eq!(split("cons/"), vec!["cons", ""], "a trailing slash");
        assert_eq!(split("/cons"), vec!["", "cons"], "a doubled slash");
    }

    #[test]
    fn permissions() {
        // The root is 0555: its group slot is read and execute only.
        assert!(perm_ok(0o555, OREAD), "a root read");
        assert!(!perm_ok(0o555, OWRITE), "a root write");
        assert!(!perm_ok(0o555, ORDWR), "a root read and write");
        // cons is 0220: only the write.
        assert!(!perm_ok(0o220, OREAD), "a cons read");
        assert!(perm_ok(0o220, OWRITE), "a cons write");
        assert!(!perm_ok(0o220, ORDWR), "a cons read and write");
        // A mode without the group slot grants nothing.
        assert!(!perm_ok(0o500, OREAD), "owner bits only");
        assert!(!perm_ok(0o005, OREAD), "other bits only");
        assert!(!perm_ok(0o000, OREAD), "no bits");
    }

    #[test]
    fn opens() {
        // A bare #c opens the root channel.
        let c = open_name("#c", OREAD).expect("a bare #c");
        assert_eq!(c.qid().path, 0);
        assert_eq!(c.qid().kind, QTDIR);
        assert_eq!(c.mode(), OREAD);
        assert_eq!(c.length(), 0, "the root's length");
        assert_eq!(c.offset(), 0, "an opened channel's offset");

        // #c/cons, in the access its 0220 grants, and the flags strip.
        let c = open_name("#c/cons", OWRITE | OTRUNC).expect("#c/cons");
        assert_eq!(c.qid().path, 1);
        assert_eq!(c.qid().kind, QTFILE);
        assert_eq!(c.mode(), OWRITE, "the flags strip");
        assert_eq!(c.length(), 0, "cons's length");

        // The permissions that fail, as the modes say: a write of a
        // directory fails the group check, as Plan 9's devopen orders it,
        // before its directory arm.
        assert_eq!(err("#c/cons", OREAD), EPERM, "a cons read");
        assert_eq!(err("#c/cons", ORDWR), EPERM, "a cons read and write");
        assert_eq!(err("#c", OWRITE), EPERM, "a directory write");
        assert_eq!(err("#c", ORDWR), EPERM, "a directory read and write");

        // A mode openmode can't reduce.
        assert_eq!(err("#c/cons", OWRITE | 0x100), EBADARG, "above OEXEC");
    }

    #[test]
    fn walks() {
        // The walk's error ordering, as Plan 9's devwalk has it.
        assert_eq!(err("#c/nope", OREAD), ENONEXIST, "a missing name");
        assert_eq!(err("#c/cons/", OWRITE), ENONEXIST, "a trailing slash");
        assert_eq!(err("#c//cons", OWRITE), ENONEXIST, "a doubled slash");
        assert_eq!(err("#c/cons/extra", OREAD), ENOTDIR, "past the file");

        // A bad # prefix.
        assert_eq!(err("#x/cons", OWRITE), EBADSHARP, "an unknown device");
        assert_eq!(err("cons", OWRITE), EBADSHARP, "no #");
    }

    #[test]
    fn writes() {
        fake_cons();
        let c = open_name("#c/cons", OWRITE).expect("cons");
        assert_eq!(c.dev().write(&c, b"hi\n", 0).expect("a write"), 3);
        assert_eq!(cons_output(), b"hi\r\n", "through the console");

        // Every read fails, until the read path arrives.
        let mut buf = [0u8; 4];
        assert_eq!(c.dev().read(&c, &mut buf, 0), Err(ENOTSUP), "a cons read");

        let root = open_name("#c", OREAD).expect("the root");
        assert_eq!(root.dev().read(&root, &mut buf, 0), Err(ENOTSUP), "a directory read");
    }
}
