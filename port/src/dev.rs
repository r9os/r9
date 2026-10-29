//! Devices, and the resolution of a name to an open channel, as Plan 9's
//! dev.c and namec in chan.c.

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::chan::{self, Chan, OREAD, QTDIR};
use crate::console;
use crate::error::{EBADSHARP, ENONEXIST, ENOTDIR};

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
static DEVS: &[&dyn Dev] = &[&console::CONS];

/// The device with dc `dc`, as Plan 9's devno scans its table.
fn devno(dc: char) -> Option<&'static dyn Dev> {
    DEVS.iter().find(|d| d.dc() == dc).copied()
}

/// The names in the root that stand for a device's root, as Plan 9's
/// /lib/namespace binds #c on /dev.  There's no bind yet, so every process
/// sees this one table.
static BINDS: &[(&str, &dyn Dev)] = &[("dev", &console::CONS)];

/// The device and path a # name names, from what follows its #, as the #
/// branch of Plan 9's namec: a device's letter, then either nothing or a /
/// and the path from the device's root; anything else is EBADSHARP, as the
/// device is unknown.
fn devpath(name: &str) -> crate::Result<(&'static dyn Dev, &str)> {
    let Some(dc) = name.chars().next() else {
        return Err(EBADSHARP);
    };
    let path = match &name[dc.len_utf8()..] {
        "" => "",
        p => p.strip_prefix('/').ok_or(EBADSHARP)?,
    };
    devno(dc).ok_or(EBADSHARP).map(|d| (d, path))
}

/// The elements of a path, and whether it must name a directory, as Plan 9's
/// parsename: the names between its slashes, skipping empty ones and ., as
/// its skipslash does, so a doubled slash is a single one.  A path that ends
/// after its last element, in / or /., must name a directory.
fn parse(path: &str) -> (Vec<&str>, bool) {
    let elems = path.split('/').filter(|e| !matches!(*e, "" | ".")).collect();
    let mustbedir = matches!(path.rsplit('/').next(), Some("" | "."));
    (elems, mustbedir)
}

/// The device a name starts from, the elements to walk from its root, and
/// whether they must name a directory: a # name's device, or for a name from
/// the root, the device bound on its first element.  Any other name is
/// ENONEXIST, as there's nothing else in the root, and no current directory.
fn resolve(name: &str) -> crate::Result<(&'static dyn Dev, Vec<&str>, bool)> {
    if let Some(rest) = name.strip_prefix('#') {
        let (dev, path) = devpath(rest)?;
        let (elems, mustbedir) = parse(path);
        return Ok((dev, elems, mustbedir));
    }
    if !name.starts_with('/') {
        return Err(ENONEXIST);
    }
    let (mut elems, mustbedir) = parse(name);
    let first = if elems.is_empty() { None } else { Some(elems.remove(0)) };
    let dev = BINDS.iter().find(|(n, _)| Some(*n) == first).ok_or(ENONEXIST)?.1;
    Ok((dev, elems, mustbedir))
}

/// The channel `name` opens for the access `omode`, as Plan 9's namec: the
/// mode is checked first, as Plan 9's openmode, then the name resolved, the
/// device attached, the path walked, and the channel opened.
pub(crate) fn open_name(name: &str, omode: u32) -> crate::Result<Arc<Chan>> {
    let omode = chan::openmode(omode)?;
    let (dev, elems, mustbedir) = resolve(name)?;
    let c = dev.attach()?;
    let c = dev.walk(&c, &elems)?;
    if mustbedir && c.qid().kind != QTDIR {
        return Err(ENOTDIR);
    }
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

    /// The device and path what follows a # names, as devpath does.
    fn path_of(name: &str) -> Result<(char, &str), &'static str> {
        devpath(name).map(|(d, p)| (d.dc(), p))
    }

    /// The error a name's open fails with.
    fn err(name: &str, omode: u32) -> &'static str {
        open_name(name, omode).err().expect("the open to fail")
    }

    #[test]
    fn devpaths() {
        assert_eq!(path_of("c"), Ok(('c', "")), "bare");
        assert_eq!(path_of("c/cons"), Ok(('c', "cons")));
        assert_eq!(path_of("c/cons/"), Ok(('c', "cons/")));
        assert_eq!(path_of("c//cons"), Ok(('c', "/cons")));
        assert_eq!(path_of("c/"), Ok(('c', "")));
        assert_eq!(path_of("x"), Err(EBADSHARP), "an unknown device");
        assert_eq!(path_of("ccons"), Err(EBADSHARP), "no slash after the letter");
        assert_eq!(path_of(""), Err(EBADSHARP), "no letter");
    }

    #[test]
    fn parses() {
        assert_eq!(parse(""), (vec![], true), "an empty path is the root");
        assert_eq!(parse("cons"), (vec!["cons"], false));
        assert_eq!(parse("cons/"), (vec!["cons"], true), "a trailing slash");
        assert_eq!(parse("cons/."), (vec!["cons"], true), "a trailing .");
        assert_eq!(parse("/cons"), (vec!["cons"], false), "a doubled slash");
        assert_eq!(parse("./cons"), (vec!["cons"], false), "a .");
        assert_eq!(parse("a//./b"), (vec!["a", "b"], false));
        assert_eq!(parse("../cons"), (vec!["..", "cons"], false), "a .. is kept");
        assert_eq!(parse("//./"), (vec![], true));
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
        assert_eq!(err("#c/..", OREAD), ENONEXIST, "no ..");
        assert_eq!(err("#c/cons/extra", OREAD), ENOTDIR, "past the file");

        // A trailing slash or . must name a directory, as Plan 9's namec
        // checks after the walk; a doubled slash or a . is skipped.
        assert_eq!(err("#c/cons/", OWRITE), ENOTDIR, "a trailing slash");
        assert_eq!(err("#c/cons/.", OWRITE), ENOTDIR, "a trailing .");
        open_name("#c//cons", OWRITE).expect("a doubled slash");
        open_name("#c/./cons", OWRITE).expect("a .");
        open_name("#c/", OREAD).expect("the root, with a trailing slash");

        // A bad # prefix.
        assert_eq!(err("#x/cons", OWRITE), EBADSHARP, "an unknown device");
    }

    /// The device and elements a name resolves to, as resolve does.
    fn resolved(name: &str) -> Result<(char, Vec<&str>), &'static str> {
        resolve(name).map(|(d, e, _)| (d.dc(), e))
    }

    #[test]
    fn resolves() {
        assert_eq!(resolved("#c/cons"), Ok(('c', vec!["cons"])), "a # name");
        assert_eq!(resolved("#x/cons"), Err(EBADSHARP), "an unknown device");
        assert_eq!(resolved("#"), Err(EBADSHARP), "no letter");
        assert_eq!(resolved("/dev"), Ok(('c', vec![])), "the bound directory");
        assert_eq!(resolved("/dev/"), Ok(('c', vec![])));
        assert_eq!(resolved("/dev/cons"), Ok(('c', vec!["cons"])));
        assert_eq!(resolved("//dev//cons"), Ok(('c', vec!["cons"])), "doubled slashes");
        assert_eq!(resolved("/./dev/./cons"), Ok(('c', vec!["cons"])), "dots");
        assert_eq!(resolved("/devcons"), Err(ENONEXIST), "not below /dev");
        assert_eq!(resolved("/"), Err(ENONEXIST), "no root");
        assert_eq!(resolved("/env"), Err(ENONEXIST), "nothing bound there");
        assert_eq!(resolved("cons"), Err(ENONEXIST), "no current directory");
        assert_eq!(resolved(""), Err(ENONEXIST), "nothing");
    }

    #[test]
    fn opens_through_dev() {
        fake_cons();
        let c = open_name("/dev/cons", OWRITE).expect("/dev/cons");
        assert_eq!(c.qid().path, 1, "the same file as #c/cons");
        assert_eq!(c.dev().write(&c, b"dev\n", 0).expect("a write"), 4);
        assert_eq!(cons_output(), b"dev\r\n", "through the console");

        let root = open_name("/dev", OREAD).expect("/dev");
        assert_eq!(root.qid().kind, QTDIR, "the console's root");

        assert_eq!(err("/dev/cons", OREAD), EPERM, "cons's permissions");
        assert_eq!(err("/dev/nope", OWRITE), ENONEXIST, "a missing name");
        assert_eq!(err("/dev/cons/x", OWRITE), ENOTDIR, "past the file");
        assert_eq!(err("cons", OWRITE), ENONEXIST, "no current directory");
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
