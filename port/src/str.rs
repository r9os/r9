//! Strings the kernel holds in fixed arrays.

/// Copy `src` into `dst`, NUL-terminated, and zero the rest of `dst`, as
/// Plan 9's kstrcpy: if it's longer than `N - 1` bytes, as much of
/// it as fits with "...", cut at a character boundary.  `dst` must hold the
/// "..." and its NUL, which is checked at compile time.
pub(crate) fn kstrcpy<const N: usize>(dst: &mut [u8; N], src: &str) {
    const DOTS: &[u8] = b"...";
    const { assert!(N > DOTS.len(), "kstrcpy: too short for the dots") };
    dst.fill(0);
    if src.len() < N {
        dst[..src.len()].copy_from_slice(src.as_bytes());
    } else {
        let n = src.floor_char_boundary(N - 1 - DOTS.len());
        dst[..n].copy_from_slice(&src.as_bytes()[..n]);
        dst[n..n + DOTS.len()].copy_from_slice(DOTS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kstrcpy_copies_a_string_that_fits() {
        let mut dst = [b'x'; 8];
        kstrcpy(&mut dst, "factor");
        assert_eq!(&dst, b"factor\0\0");
        kstrcpy(&mut dst, "1234567");
        assert_eq!(&dst, b"1234567\0", "the longest");
    }

    #[test]
    fn kstrcpy_cuts_a_longer_string_with_dots() {
        let mut dst = [b'x'; 8];
        kstrcpy(&mut dst, "12345678");
        assert_eq!(&dst, b"1234...\0");
        // "é" is two bytes, and straddles the cut: it goes whole.
        kstrcpy(&mut dst, "123é5678");
        assert_eq!(&dst, b"123...\0\0");
        let mut dst = [b'x'; 4];
        kstrcpy(&mut dst, "factor");
        assert_eq!(&dst, b"...\0", "the shortest");
    }
}
