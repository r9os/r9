//! Devices, and the resolution of a name to an open channel, as Plan 9's
//! dev.c and namec in chan.c.

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::chan::{self, Chan, OCEXEC, OEXEC, OREAD, QTDIR};
use crate::{devcons, devroot};
use crate::error::{EBADSHARP, EEMPTYNAME, EEXECDIR, ENONEXIST, ENOTDIR, Error};
use sys::ERRMAX;

/// A device's operations for the files it holds, as Plan 9's Device, with
/// the six this kernel needs; Plan 9's other eleven have no caller until the
/// plan reaches them.  The read and write slices are kernel-resident: the
/// copy to and from user memory is the syscall layer's, and it's generic over
/// the architecture, so a device never touches user memory.
pub trait Dev: Sync {
    /// The device's letter, the character after # in its files' names.
    fn dc(&self) -> char;

    /// A fresh channel on the device's root directory, unopened, as Plan 9's
    /// attach, given the spec that followed the device's letter in its #
    /// name.  Plan 9's devattach records it in the channel's name, which
    /// r9's channels don't have, so a device that has no use for it ignores
    /// it.
    fn attach(&self, spec: &str) -> crate::Result<Arc<Chan>>;

    /// Descend the name elements and return a fresh channel on the file they
    /// name, unopened, as Plan 9's namec makes a channel at each step of its
    /// walk.  A walk of no elements is a no-op.
    fn walk(&self, c: &Arc<Chan>, elems: &[&str]) -> crate::Result<Arc<Chan>>;

    /// Open the file in `omode`, the mode as given, without OCEXEC, and
    /// return the opened channel, as Plan 9's devopen: the permission check
    /// of the access the mode asks for, OEXEC's included, the offset to
    /// zero, a directory OREAD only, and openmode's reduction of the mode
    /// and the file's length recorded.
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
static DEVS: &[&dyn Dev] = &[&devroot::ROOT, &devcons::CONS];

/// The device with dc `dc`, as Plan 9's devno scans its table.
fn devno(dc: char) -> Option<&'static dyn Dev> {
    DEVS.iter().find(|d| d.dc() == dc).copied()
}

/// The names in the root and the # names they stand for, as Plan 9's
/// /lib/namespace binds #c on /dev, and its boot binds /bin.  There's no
/// bind yet, so every process sees this one table.
static BINDS: &[(&str, &str)] = &[("bin", "#//boot"), ("dev", "#c")];

/// The device, attach spec and path a # name names, from what follows its
/// #, as the # branch of Plan 9's namec: a device's letter, then the spec,
/// up to the next /, then the path from the device's root after it.  An
/// unknown letter, or none, is EBADSHARP.
fn devpath(name: &str) -> crate::Result<(&'static dyn Dev, &str, &str)> {
    let Some(dc) = name.chars().next() else {
        return Err(EBADSHARP);
    };
    let rest = &name[dc.len_utf8()..];
    let (spec, path) = rest.split_once('/').unwrap_or((rest, ""));
    devno(dc).ok_or(EBADSHARP).map(|d| (d, spec, path))
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

/// A name's start, as resolve finds it: the channel to walk from, the
/// elements to walk, whether they must name a directory, and the length, in
/// bytes, of the part of the name walked to reach it.
struct Start<'a> {
    c: Arc<Chan>,
    elems: Vec<&'a str>,
    mustbedir: bool,
    walked: usize,
}

/// Where a name starts: a # name's device's root, or for a name from the
/// root, the directory bound on its first element.  Any other name is
/// ENONEXIST at its first element, as there's nothing else in the root, and
/// no current directory.
fn resolve(name: &str) -> Result<Start<'_>, Error> {
    if let Some(rest) = name.strip_prefix('#') {
        let (c, elems, mustbedir) = sharp(rest)?;
        return Ok(Start { c, elems, mustbedir, walked: 0 });
    }
    let (mut elems, mustbedir) = parse(name);
    let Some(&first) = elems.first() else {
        return Err(ENONEXIST.into());
    };
    let walked = end(name, first);
    let bound = if name.starts_with('/') { BINDS.iter().find(|(n, _)| *n == first) } else { None };
    let (_, target) = bound.ok_or_else(|| nameerror(name, walked, ENONEXIST))?;
    let (c, under, _) = sharp(target.strip_prefix('#').ok_or(EBADSHARP)?)?;
    let c = c.dev().walk(&c, &under)?;
    elems.remove(0);
    Ok(Start { c, elems, mustbedir, walked })
}

/// The root of the device a # name names, from what follows its #, the
/// elements of its path, and whether they must name a directory.
fn sharp(rest: &str) -> crate::Result<(Arc<Chan>, Vec<&str>, bool)> {
    let (dev, spec, path) = devpath(rest)?;
    let (elems, mustbedir) = parse(path);
    Ok((dev.attach(spec)?, elems, mustbedir))
}

/// The end of `elem`, a slice of `name`, as an offset in `name`, as Plan 9's
/// parsename records each element's.
fn end(name: &str, elem: &str) -> usize {
    let start = elem.as_ptr() as usize - name.as_ptr() as usize;
    debug_assert!(start + elem.len() <= name.len(), "end: {elem:?} isn't a slice of {name:?}");
    start + elem.len()
}

/// The channel `name` opens in `omode`, as Plan 9's namec: the mode is
/// checked first, as Plan 9's sysopen does with openmode, then an empty name
/// refused, as namec's first check, then the name resolved, the device
/// attached, the path walked an element at a time, and the channel opened in
/// the mode without OCEXEC, which is recorded on the channel.  A directory
/// can't be opened OEXEC.  An error from the walk or after it shows the name as namec's
/// does: up to the element that failed, or for an element that isn't a
/// directory, up to that element; past the walk, the whole name; and none if
/// the name has no elements.
pub(crate) fn open_name(name: &str, omode: u32) -> Result<Arc<Chan>, Error> {
    chan::openmode(omode)?;
    if name.is_empty() {
        return Err(EEMPTYNAME.into());
    }
    let Start { mut c, elems, mustbedir, mut walked } = resolve(name)?;
    let dev = c.dev();
    for elem in elems {
        if c.qid().kind != QTDIR {
            return Err(nameerror(name, walked, ENOTDIR));
        }
        walked = end(name, elem);
        c = dev.walk(&c, &[elem]).map_err(|err| nameerror(name, walked, err))?;
    }
    let whole = if walked == 0 { 0 } else { name.len() };
    if mustbedir && c.qid().kind != QTDIR {
        return Err(nameerror(name, whole, ENOTDIR));
    }
    if omode & 3 == OEXEC && c.qid().kind == QTDIR {
        return Err(nameerror(name, whole, EEXECDIR));
    }
    let c = dev.open(&c, omode & !OCEXEC).map_err(|err| nameerror(name, whole, err))?;
    if omode & OCEXEC != 0 {
        c.set_cexec();
    }
    Ok(c)
}

/// The error `err` from resolving `name`, showing its first `len` bytes, as
/// Plan 9's namelenerror: all of them if they're short enough beside `err`;
/// else ... and as many of their last elements as are; else, for a last
/// element too long by itself, its last ERRMAX/4 bytes.  None is shown for
/// a `len` of 0.
fn nameerror(name: &str, len: usize, err: &'static str) -> Error {
    if len == 0 {
        return Error::Str(err);
    }
    let short = |n: usize| n < ERRMAX / 3 || n + err.len() < 2 * ERRMAX / 3;
    let name = &name[..len];
    if short(len) {
        return Error::Name(name.into(), err);
    }
    // Move back a slash at a time while what follows it is short enough,
    // and show what followed the last slash that was.
    let mut next = len;
    let mut start;
    loop {
        start = next;
        next = name[..start].rfind('/').unwrap_or(0);
        if !short(len - next) {
            break;
        }
    }
    if start == len {
        start = name.ceil_char_boundary(len - ERRMAX / 4);
    }
    Error::Name(format!("...{}", &name[start..]), err)
}

/// Copy the part of `s` from `off` that fits into `buf`, and return its
/// length, as Plan 9's readstr: nothing at or past its end.
pub(crate) fn readstr(off: u64, buf: &mut [u8], s: &[u8]) -> u64 {
    let rest = usize::try_from(off).ok().and_then(|o| s.get(o..)).unwrap_or_default();
    let n = rest.len().min(buf.len());
    buf[..n].copy_from_slice(&rest[..n]);
    n as u64
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
    use alloc::string::String;

    use crate::chan::{OWRITE, ORDWR, OTRUNC, QTDIR, QTFILE};
    use crate::error::{ENONEXIST, ENOTSUP};
    use crate::fake::{cons_output, fake_cons};

    /// The device and path what follows a # names, as devpath does.
    fn path_of(name: &str) -> Result<(char, &str, &str), &'static str> {
        devpath(name).map(|(d, s, p)| (d.dc(), s, p))
    }

    /// The text of the error a name's open fails with.
    fn err(name: &str, omode: u32) -> String {
        text(&open_name(name, omode).err().expect("the open to fail"))
    }

    /// An error's text, as the error string would hold it.
    fn text(err: &Error) -> String {
        let mut buf = [0u8; ERRMAX - 1];
        let n = err.text(&mut buf);
        String::from_utf8(buf[..n].to_vec()).expect("UTF-8")
    }

    #[test]
    fn devpaths() {
        assert_eq!(path_of("c"), Ok(('c', "", "")), "bare");
        assert_eq!(path_of("c/cons"), Ok(('c', "", "cons")));
        assert_eq!(path_of("c/cons/"), Ok(('c', "", "cons/")));
        assert_eq!(path_of("c//cons"), Ok(('c', "", "/cons")));
        assert_eq!(path_of("c/"), Ok(('c', "", "")));
        assert_eq!(path_of("/"), Ok(('/', "", "")), "the root device");
        assert_eq!(path_of("//boot"), Ok(('/', "", "boot")));
        // What follows the letter, up to a /, is the attach spec.
        assert_eq!(path_of("ccons"), Ok(('c', "cons", "")), "a spec");
        assert_eq!(path_of("cspec/time"), Ok(('c', "spec", "time")), "a spec and a path");
        assert_eq!(path_of("/boot"), Ok(('/', "boot", "")), "#/'s spec");
        assert_eq!(path_of("x"), Err(EBADSHARP), "an unknown device");
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
        // pid is 0444: only the read.
        assert!(perm_ok(0o444, OREAD), "a pid read");
        assert!(!perm_ok(0o444, OWRITE), "a pid write");
        assert!(!perm_ok(0o444, ORDWR), "a pid read and write");
        // cons is 0660: both.
        assert!(perm_ok(0o660, OREAD), "a cons read");
        assert!(perm_ok(0o660, OWRITE), "a cons write");
        assert!(perm_ok(0o660, ORDWR), "a cons read and write");
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
        // An error past the walk shows the whole name, and #c alone has no
        // elements to show.
        assert_eq!(err("#c/pid", OWRITE), "'#c/pid' permission denied", "a pid write");
        assert_eq!(err("#c/pid", ORDWR), "'#c/pid' permission denied", "a pid read and write");
        assert_eq!(err("#c", OWRITE), "permission denied", "a directory write");
        assert_eq!(err("#c", ORDWR), "permission denied", "a directory read and write");

        // OEXEC asks the device for execute permission, which cons's 0660
        // doesn't grant, and a directory can't be opened OEXEC, as namec
        // checks after the walk.  A directory's open in any mode but OREAD
        // is Eperm, flags included, as devopen checks the mode as given.
        assert_eq!(err("#c/cons", OEXEC), "'#c/cons' permission denied", "OEXEC");
        assert_eq!(err("/dev", OEXEC), "'/dev' cannot exec directory", "a directory");
        assert_eq!(err("#c", OREAD | OTRUNC), "permission denied", "a directory truncated");

        // OCEXEC is recorded on the channel, and doesn't reach the device.
        assert!(open_name("#c/cons", OWRITE | OCEXEC).expect("OCEXEC").cexec(), "OCEXEC");
        assert!(!open_name("#c/cons", OWRITE).expect("cons").cexec(), "no OCEXEC");

        // An empty name, after the mode, as sysopen's openmode comes
        // before namec's first check.
        assert_eq!(err("", OREAD), "empty file name", "an empty name");
        assert_eq!(err("", OWRITE | 0x100), "bad arg in system call", "the mode first");

        // A mode openmode can't reduce, before the name is resolved.
        assert_eq!(err("#c/cons", OWRITE | 0x100), "bad arg in system call", "above OEXEC");
    }

    #[test]
    fn walks() {
        // The walk's error ordering, as Plan 9's devwalk has it, showing
        // the name up to the element that failed, or up to the file walked
        // past, as namec does.
        assert_eq!(err("#c/nope", OREAD), "'#c/nope' file does not exist", "a missing name");
        assert_eq!(err("#c/cons/extra", OREAD), "'#c/cons' not a directory", "past the file");

        // A trailing slash or . must name a directory, as Plan 9's namec
        // checks after the walk, showing the whole name; a doubled slash or
        // a . is skipped.
        assert_eq!(err("#c/cons/", OWRITE), "'#c/cons/' not a directory", "a trailing slash");
        assert_eq!(err("#c/cons/.", OWRITE), "'#c/cons/.' not a directory", "a trailing .");
        open_name("#c//cons", OWRITE).expect("a doubled slash");
        open_name("#c/./cons", OWRITE).expect("a .");
        open_name("#c/", OREAD).expect("the root, with a trailing slash");

        // A bad # prefix, before there's a name to show.
        assert_eq!(err("#x/cons", OWRITE), "unknown device in # filename", "an unknown device");
    }

    /// The device and elements a name resolves to, as resolve does, or its
    /// error's text.
    fn resolved(name: &str) -> Result<(char, Vec<&str>), String> {
        resolve(name).map(|s| (s.c.dev().dc(), s.elems)).map_err(|e| text(&e))
    }

    #[test]
    fn resolves() {
        assert_eq!(resolved("#c/cons"), Ok(('c', vec!["cons"])), "a # name");
        let badsharp = || Err(EBADSHARP.into());
        assert_eq!(resolved("#x/cons"), badsharp(), "an unknown device");
        assert_eq!(resolved("#"), badsharp(), "no letter");
        assert_eq!(resolved("/dev"), Ok(('c', vec![])), "the bound directory");
        assert_eq!(resolved("/dev/"), Ok(('c', vec![])));
        assert_eq!(resolved("/dev/cons"), Ok(('c', vec!["cons"])));
        assert_eq!(resolved("//dev//cons"), Ok(('c', vec!["cons"])), "doubled slashes");
        assert_eq!(resolved("/./dev/./cons"), Ok(('c', vec!["cons"])), "dots");
        assert_eq!(resolved("/bin/echo"), Ok(('/', vec!["echo"])), "bound below a device's root");
        assert_eq!(resolved("#//boot/echo"), Ok(('/', vec!["boot", "echo"])), "the root device");
        // namec takes what follows #/ up to the next slash as an attach
        // spec, which #/ ignores, as Plan 9's rootattach: #/boot is #/.
        assert_eq!(resolved("#/boot"), Ok(('/', vec![])), "a spec");
        assert_eq!(resolved("#ccons/time"), Ok(('c', vec!["time"])), "#c's");
        // A name that isn't bound fails at its first element, which is
        // shown; one with no elements shows nothing.
        let missing = |n: &str| Err(alloc::format!("'{n}' file does not exist"));
        assert_eq!(resolved("/devcons"), missing("/devcons"), "not below /dev");
        assert_eq!(resolved("/env/x"), missing("/env"), "nothing bound there");
        assert_eq!(resolved("cons"), missing("cons"), "no current directory");
        assert_eq!(resolved("/"), Err(ENONEXIST.into()), "no root");
        assert_eq!(resolved(""), Err(ENONEXIST.into()), "nothing");
    }

    #[test]
    fn binds_resolve() {
        for (name, target) in BINDS {
            let path = alloc::format!("/{name}");
            let bound = resolve(&path).expect("a bound name");
            let c = open_name(target, OREAD).expect("its target");
            assert_eq!(bound.c.dev().dc(), c.dev().dc(), "{name}'s device");
            assert_eq!(bound.c.qid(), c.qid(), "{name}'s directory");
        }
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

        assert_eq!(err("/dev/pid", OWRITE), "'/dev/pid' permission denied", "pid's permissions");
        assert_eq!(err("/dev", OWRITE), "'/dev' permission denied", "the bound directory");
        assert_eq!(err("/dev/nope", OWRITE), "'/dev/nope' file does not exist", "a missing name");
        assert_eq!(err("/dev/nope/x", OWRITE), "'/dev/nope' file does not exist", "up to the element");
        assert_eq!(err("/dev/cons/x", OWRITE), "'/dev/cons' not a directory", "past the file");
        assert_eq!(err("cons", OWRITE), "'cons' file does not exist", "no current directory");
    }

    #[test]
    fn dotdot_walks_up_a_devices_table() {
        // .. is a no-op at a device's root, as Plan 9's devgen gives it.
        assert_eq!(open_name("#c/..", OREAD).expect("#c/..").qid().path, 0, "the root");
        let c = open_name("#c/../cons", OWRITE).expect("#c/../cons");
        assert_eq!(c.qid().path, 1, "cons, from the root's ..");
        assert_eq!(open_name("/dev/../time", OREAD).expect("/dev/../time").qid().path, 2);
        // From #/'s boot, it's #/'s root, and from there, the root again.
        for name in ["#//boot/..", "#//boot/../..", "/bin/.."] {
            let c = open_name(name, OREAD).expect(name);
            assert_eq!((c.dev().dc(), c.qid().path, c.qid().kind), ('/', 0, QTDIR), "{name}");
        }
        assert!(open_name("#//boot/../boot", OREAD).is_ok(), "boot, back down");
    }

    #[test]
    fn long_names_are_shortened() {
        // A name short enough beside its error is shown whole.
        let short = nameerror("/dev/nope", 9, ENONEXIST);
        assert_eq!(short, Error::Name("/dev/nope".into(), ENONEXIST));

        // A long one shows ... and as many of its last elements as fit:
        // twenty elements of ten bytes show their last six, sixty bytes,
        // the most beside the error's nineteen within 2*ERRMAX/3.
        let name: String = (0..20).map(|i| alloc::format!("/element{i:02}")).collect();
        let shown = nameerror(&name, name.len(), ENONEXIST);
        assert_eq!(shown, Error::Name(alloc::format!("...{}", &name[140..]), ENONEXIST));

        // A last element too long by itself shows its last ERRMAX/4 bytes.
        let name = alloc::format!("/dev/{}", "x".repeat(100));
        let chopped = alloc::format!("...{}", "x".repeat(ERRMAX / 4));
        assert_eq!(nameerror(&name, name.len(), ENONEXIST), Error::Name(chopped, ENONEXIST));
        assert_eq!(
            err(&name, OREAD),
            alloc::format!("'...{}' file does not exist", "x".repeat(ERRMAX / 4)),
            "through open"
        );

        // A shortened name falls back a character at a time to a boundary.
        let name = alloc::format!("/dev/{}", "é".repeat(50));
        let Error::Name(shown, _) = nameerror(&name, name.len(), ENONEXIST) else {
            panic!("a name error");
        };
        assert!(shown.len() <= ERRMAX / 4 + 3 && shown.ends_with('é'));
    }

    #[test]
    fn writes() {
        fake_cons();
        let c = open_name("#c/cons", OWRITE).expect("cons");
        assert_eq!(c.dev().write(&c, b"hi\n", 0).expect("a write"), 3);
        assert_eq!(cons_output(), b"hi\r\n", "through the console");

        // A directory's read isn't supported yet.
        let mut buf = [0u8; 4];
        let root = open_name("#c", OREAD).expect("the root");
        assert_eq!(root.dev().read(&root, &mut buf, 0), Err(ENOTSUP), "a directory read");
    }
}
