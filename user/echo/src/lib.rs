//! echo's logic, from Plan 9's echo.c, apart from its system calls, so that
//! it can be tested on the host.

#![cfg_attr(not(test), no_std)]

/// Split `argv` into whether -n was given, which it is only as argv[1], and
/// the words to print.
pub fn split<'a, 'b>(argv: &'b [&'a [u8]]) -> (bool, &'b [&'a [u8]]) {
    let nflag = argv.get(1) == Some(&&b"-n"[..]);
    let words = argv.get(1 + nflag as usize..).unwrap_or(&[]);
    (nflag, words)
}

/// The size of buffer echo.c allocates for `words`: each word and a space or
/// newline after it, and a byte more for strcpy's NUL.
pub fn size(words: &[&[u8]]) -> usize {
    1 + words.iter().map(|w| w.len() + 1).sum::<usize>()
}

/// Write `words` into `buf`, separated by spaces, and then a newline unless
/// `nflag`.  Return the length written.  `buf` must hold size(words) bytes.
pub fn fill(words: &[&[u8]], nflag: bool, buf: &mut [u8]) -> usize {
    let mut p = 0;
    for (i, w) in words.iter().enumerate() {
        buf[p..p + w.len()].copy_from_slice(w);
        p += w.len();
        if i < words.len() - 1 {
            buf[p] = b' ';
            p += 1;
        }
    }
    if !nflag {
        buf[p] = b'\n';
        p += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What echo with `argv` writes.
    fn echo(argv: &[&[u8]]) -> Vec<u8> {
        let (nflag, words) = split(argv);
        let mut buf = vec![0; size(words)];
        let n = fill(words, nflag, &mut buf);
        assert!(n <= buf.len());
        buf.truncate(n);
        buf
    }

    #[test]
    fn echoes() {
        assert_eq!(echo(&[b"echo"]), b"\n", "no arguments");
        assert_eq!(echo(&[]), b"\n", "no argv[0]");
        assert_eq!(echo(&[b"echo", b"-n"]), b"", "-n alone");
        assert_eq!(echo(&[b"echo", b"a", b"b"]), b"a b\n");
        assert_eq!(echo(&[b"echo", b"-n", b"a", b"b"]), b"a b");
        assert_eq!(echo(&[b"echo", b"a", b"-n"]), b"a -n\n", "-n not first");
        assert_eq!(echo(&[b"echo", b"-n", b"-n"]), b"-n", "a second -n");
        assert_eq!(echo(&[b"echo", b"a", b"", b"b"]), b"a  b\n", "an empty argument");
    }

    #[test]
    fn size_is_echo_c_len() {
        // len = 1 + sum(strlen(argv[i]) + 1)
        assert_eq!(size(&[]), 1);
        assert_eq!(size(&[b"hello", b"world"]), 1 + 6 + 6);
    }
}
