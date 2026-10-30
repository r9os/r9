//! Error strings, as Plan 9's port/error.h, and the errors a syscall reports.

use alloc::string::String;

pub const EBADARG: &str = "bad arg in system call";
pub const EBADFD: &str = "fd out of range or not open";
pub const EBADSHARP: &str = "unknown device in # filename";
pub const EBADTIMECTL: &str = "bad time control";
pub const EBADUSEFD: &str = "inappropriate use of fd";
pub const EBADEXEC: &str = "exec header invalid";
pub const EEXECDIR: &str = "cannot exec directory";
pub const EGREG: &str = "jmk added reentrancy for threads";
pub const EINUSE: &str = "device or object already in use";
pub const EISDIR: &str = "file is a directory";
pub const ENAMETOOLONG: &str = "name too long";
pub const ENEGOFF: &str = "negative i/o offset";
pub const ENOCHILD: &str = "no living children";
pub const ENOFD: &str = "no free file descriptors";
pub const ENONEXIST: &str = "file does not exist";
pub const ENOTSUP: &str = "not supported";
pub const ENOTDIR: &str = "not a directory";
pub const ENOVMEM: &str = "virtual memory allocation failed";
pub const EPERM: &str = "permission denied";
pub const ESOVERLAP: &str = "segments overlap";

/// An error as a syscall reports it: one of the strings above, or one from
/// resolving a name, which shows the name, as Plan 9's namelenerror does.
#[derive(Debug, PartialEq)]
pub enum Error {
    Str(&'static str),
    /// The part of the name to show, shortened to fit, and the error.
    Name(String, &'static str),
}

impl From<&'static str> for Error {
    fn from(err: &'static str) -> Error {
        Error::Str(err)
    }
}

impl Error {
    /// Write the error's text into `buf`, as much as fits, ending on a
    /// character, and return its length, as Plan 9's snprint does.  A name
    /// is quoted as Plan 9's %#q quotes it, always, with its quotes doubled,
    /// and followed by a space and the error.
    pub fn text(&self, buf: &mut [u8]) -> usize {
        let mut t = Text { buf, n: 0, full: false };
        match self {
            Error::Str(err) => t.put(err),
            Error::Name(name, err) => {
                t.put("'");
                for (i, part) in name.split('\'').enumerate() {
                    if i > 0 {
                        t.put("''");
                    }
                    t.put(part);
                }
                t.put("' ");
                t.put(err);
            }
        }
        t.n
    }
}

/// A buffer being filled with text, which stops at the first string that
/// doesn't fit, after as many of its characters as do.
struct Text<'a> {
    buf: &'a mut [u8],
    n: usize,
    full: bool,
}

impl Text<'_> {
    fn put(&mut self, s: &str) {
        if self.full {
            return;
        }
        let k = s.floor_char_boundary(s.len().min(self.buf.len() - self.n));
        self.buf[self.n..self.n + k].copy_from_slice(&s.as_bytes()[..k]);
        self.n += k;
        self.full = k < s.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(err: &Error, len: usize) -> String {
        let mut buf = alloc::vec![0u8; len];
        let n = err.text(&mut buf);
        String::from_utf8(buf[..n].to_vec()).expect("UTF-8")
    }

    #[test]
    fn texts() {
        assert_eq!(text(&Error::Str(EPERM), 128), "permission denied");
        let missing = Error::Name("/dev/nope".into(), ENONEXIST);
        assert_eq!(text(&missing, 128), "'/dev/nope' file does not exist");
        assert_eq!(text(&Error::Name("it's".into(), EPERM), 128), "'it''s' permission denied", "a quote doubled");
        assert_eq!(text(&Error::Name("plain".into(), EPERM), 128), "'plain' permission denied", "always quoted");
        assert_eq!(text(&missing, 7), "'/dev/n", "cut to fit");
        assert_eq!(text(&Error::Name("aé".into(), EPERM), 3), "'a", "on a character");
        assert_eq!(text(&Error::Name("é".into(), EPERM), 128).len(), "'é' permission denied".len());
    }
}
