//! The root device, #/, as Plan 9's devroot: a root directory holding boot,
//! which holds the programs the architecture registers, as Plan 9's kernel
//! config puts its bootdir's files there.

use alloc::sync::Arc;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::chan::{Chan, OREAD, Qid, QTDIR, QTFILE};
use crate::dev::{self, Dev};
use crate::error::{EGREG, ENONEXIST, ENOTDIR, ENOTSUP, EPERM};

/// The root device, reached as #/.
pub struct Root;

pub static ROOT: Root = Root;

/// A file in #/boot: its name and its contents, as Plan 9's addbootfile
/// takes them.
pub struct BootFile {
    pub name: &'static str,
    pub data: &'static [u8],
}

/// The qid paths of the root and boot directories, as Plan 9's Qdir and
/// Qboot, and of the first boot file: a file's is that plus its index, as
/// addlist numbers them after the boot directory's own entry.
const QDIR: u64 = 0;
const QBOOT: u64 = 0x1000;
const QFILE0: u64 = QBOOT + 2;

// A reference to the slice, as AtomicPtr holds a thin pointer, and a
// slice's is fat.
#[cfg(not(test))]
static BOOT: AtomicPtr<&'static [BootFile]> = AtomicPtr::new(ptr::null_mut());

// Tests run in parallel threads, so each registers its own files.
#[cfg(test)]
std::thread_local! {
    static BOOT: AtomicPtr<&'static [BootFile]> = const { AtomicPtr::new(ptr::null_mut()) };
}

/// Register the architecture's programs, as the files of #/boot.
pub fn init(files: &'static &'static [BootFile]) {
    let files = ptr::from_ref(files).cast_mut();
    core::cfg_select! {
        test => BOOT.with(|b| b.store(files, Ordering::Release)),
        _ => BOOT.store(files, Ordering::Release),
    }
}

/// The files of #/boot: none until the architecture registers them.
fn boot() -> &'static [BootFile] {
    let files = core::cfg_select! {
        test => BOOT.with(|b| b.load(Ordering::Acquire)),
        _ => BOOT.load(Ordering::Acquire),
    };
    // SAFETY: BOOT is null, or was set from a &'static &'static [BootFile]
    // by init.
    unsafe { files.as_ref() }.copied().unwrap_or_default()
}

/// The boot file with qid path `path`.
fn file(path: u64) -> Option<&'static BootFile> {
    let i = path.checked_sub(QFILE0)?;
    boot().get(usize::try_from(i).ok()?)
}

impl Dev for Root {
    fn dc(&self) -> char {
        '/'
    }

    fn attach(&self) -> crate::Result<Arc<Chan>> {
        Ok(Chan::new(&ROOT, Qid { path: QDIR, kind: QTDIR }))
    }

    fn walk(&self, c: &Arc<Chan>, elems: &[&str]) -> crate::Result<Arc<Chan>> {
        let mut c = c.clone();
        for elem in elems {
            let qid = match c.qid().path {
                QDIR if *elem == "boot" => Qid { path: QBOOT, kind: QTDIR },
                QBOOT => {
                    let i = boot().iter().position(|f| f.name == *elem).ok_or(ENONEXIST)?;
                    Qid { path: QFILE0 + i as u64, kind: QTFILE }
                }
                QDIR => return Err(ENONEXIST),
                _ => return Err(ENOTDIR),
            };
            c = Chan::new(&ROOT, qid);
        }
        Ok(c)
    }

    /// Open as Plan 9's rootopen, whose devopen is given no table to find
    /// the file's mode in, so no permission is checked: only a directory's
    /// open is OREAD only.
    fn open(&self, c: &Arc<Chan>, omode: u32) -> crate::Result<Arc<Chan>> {
        if c.qid().kind == QTDIR && omode != OREAD {
            return Err(EPERM);
        }
        let length = file(c.qid().path).map_or(0, |f| f.data.len() as u64);
        Ok(Chan::opened(&ROOT, *c.qid(), omode, length))
    }

    fn read(&self, c: &Arc<Chan>, buf: &mut [u8], offset: u64) -> crate::Result<u64> {
        let f = file(c.qid().path).ok_or(ENOTSUP)?;
        Ok(dev::readstr(offset, buf, f.data))
    }

    fn write(&self, _c: &Arc<Chan>, _buf: &[u8], _offset: u64) -> crate::Result<u64> {
        Err(EGREG)
    }

    fn close(&self, _c: &Chan) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chan::OWRITE;
    use crate::dev::open_name;
    use crate::error::Error;

    static FILES: &[BootFile] =
        &[BootFile { name: "echo", data: b"echo's image" }, BootFile { name: "cat", data: b"cat" }];

    /// What a read of `c` at `offset` gives, into a buffer of `n` bytes.
    fn read(c: &Arc<Chan>, offset: u64, n: usize) -> crate::Result<std::vec::Vec<u8>> {
        let mut buf = std::vec![0u8; n];
        let m = c.dev().read(c, &mut buf, offset)?;
        buf.truncate(m as usize);
        Ok(buf)
    }

    #[test]
    fn bin_reads_a_registered_file() {
        init(&FILES);
        let c = open_name("/bin/echo", OREAD).expect("/bin/echo");
        assert_eq!(c.length(), 12, "the image's length");
        assert_eq!(read(&c, 0, 100).expect("a read"), b"echo's image");
        assert_eq!(read(&c, 7, 3).expect("a read from an offset"), b"ima");
        assert_eq!(read(&c, 12, 100).expect("the end"), b"");

        let c = open_name("#//boot/cat", OREAD).expect("#//boot/cat");
        assert_eq!(read(&c, 0, 100).expect("a read"), b"cat", "the same files through #/");
    }

    #[test]
    fn boot_is_as_plan9s() {
        init(&FILES);
        // The directories are 0555, and open OREAD only.
        assert_eq!(open_name("/bin", OREAD).expect("/bin").qid().kind, QTDIR);
        assert_eq!(open_name("#/", OREAD).expect("#/").qid().kind, QTDIR);
        assert_eq!(open_name("/bin", OWRITE).err(), Some(Error::Name("/bin".into(), EPERM)));

        // A file's open checks no permission, as rootopen's, and its write
        // is Egreg, as rootwrite's.
        let c = open_name("/bin/echo", OWRITE).expect("a write open");
        assert_eq!(c.dev().write(&c, b"x", 0), Err(EGREG));

        assert_eq!(
            open_name("/bin/nosuch", OREAD).err(),
            Some(Error::Name("/bin/nosuch".into(), ENONEXIST))
        );
        assert_eq!(
            open_name("/bin/echo/x", OREAD).err(),
            Some(Error::Name("/bin/echo".into(), ENOTDIR))
        );
        assert_eq!(
            open_name("#//nosuch", OREAD).err(),
            Some(Error::Name("#//nosuch".into(), ENONEXIST)),
            "only boot is in the root"
        );
    }

    #[test]
    fn boot_is_empty_until_registered() {
        assert_eq!(
            open_name("/bin/echo", OREAD).err(),
            Some(Error::Name("/bin/echo".into(), ENONEXIST))
        );
    }
}
