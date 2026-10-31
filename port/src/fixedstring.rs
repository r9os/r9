//! A string of at most N bytes, held inline, so making or copying one never
//! allocates.

use core::{fmt, str};

/// A string of at most `N` bytes, at most 255, held inline with its length.
#[derive(Clone, Copy)]
pub struct FixedString<const N: usize> {
    buf: [u8; N],
    len: u8,
}

impl<const N: usize> FixedString<N> {
    /// The empty string.
    pub const fn empty() -> Self {
        const { assert!(N <= u8::MAX as usize) };
        Self { buf: [0; N], len: 0 }
    }

    /// `s`, cut at a character boundary if it's longer than `N` bytes.
    pub fn new(s: &str) -> Self {
        Self::with_truncation_suffix(s, "")
    }

    /// `s`, or if it's longer than `N` bytes, as much of it as fits with
    /// `suffix`, cut at a character boundary.  `suffix` must fit in `N`.
    pub fn with_truncation_suffix(s: &str, suffix: &str) -> Self {
        assert!(suffix.len() <= N, "FixedString: a suffix of {} bytes doesn't fit in {N}", suffix.len());
        let mut f = Self::empty();
        if s.len() <= N {
            f.push(s);
        } else {
            f.push(&s[..s.floor_char_boundary(N - suffix.len())]);
            f.push(suffix);
        }
        f
    }

    fn push(&mut self, s: &str) {
        let n = usize::from(self.len);
        self.buf[n..n + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len() as u8;
    }

    /// The string.
    pub fn as_str(&self) -> &str {
        str::from_utf8(&self.buf[..usize::from(self.len)]).expect("a FixedString is UTF-8")
    }
}

impl<const N: usize> fmt::Display for FixedString<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_string_that_fits() {
        assert_eq!(FixedString::<8>::empty().as_str(), "");
        assert_eq!(FixedString::<8>::new("factor").as_str(), "factor");
        assert_eq!(FixedString::<8>::new("12345678").as_str(), "12345678", "the longest");
    }

    #[test]
    fn cuts_a_longer_string() {
        assert_eq!(FixedString::<8>::new("123456789").as_str(), "12345678");
        // "é" is two bytes, and straddles the cut: it goes whole.
        assert_eq!(FixedString::<8>::new("1234567é").as_str(), "1234567");
    }

    #[test]
    fn cuts_a_longer_string_with_a_suffix() {
        let dots = |s| FixedString::<8>::with_truncation_suffix(s, "...");
        assert_eq!(dots("12345678").as_str(), "12345678", "it fits, so no suffix");
        assert_eq!(dots("123456789").as_str(), "12345...");
        assert_eq!(dots("1234é6789").as_str(), "1234...");
        assert_eq!(FixedString::<3>::with_truncation_suffix("factor", "...").as_str(), "...");
    }

    #[test]
    #[should_panic(expected = "doesn't fit")]
    fn a_suffix_longer_than_n_panics() {
        FixedString::<2>::with_truncation_suffix("factor", "...");
    }
}
