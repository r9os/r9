//! factor's logic, from Plan 9's factor.c, apart from its output, and the
//! line reading its stdin path uses, from Plan 9's libbio, so that both can
//! be tested on the host.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use libc::{modf, sqrt};

/// A Biobuf's size, as Plan 9's Bsize.
const BSIZE: usize = 8 * 1024;

/// Lines read through a buffer, as Plan 9's Brdline reads them from a
/// Biobuf.
pub struct Lines {
    buf: [u8; BSIZE],
    /// What's been read and not yet returned: buf[start..end].
    start: usize,
    end: usize,
}

impl Default for Lines {
    fn default() -> Self {
        Lines { buf: [0; BSIZE], start: 0, end: 0 }
    }
}

impl Lines {
    /// The next line, with its newline, as Brdline returns it, filling the
    /// buffer with `read` as needed: `read` reads into the slice it's given,
    /// and returns how much, 0 at the end of the file, or less on an error,
    /// as read does.  None at the end of the file, or an error, or when the
    /// buffer fills without a newline, where Brdline returns 0 too: a last
    /// line without a newline isn't returned.
    pub fn next(&mut self, mut read: impl FnMut(&mut [u8]) -> isize) -> Option<&[u8]> {
        loop {
            if let Some(i) = self.buf[self.start..self.end].iter().position(|&b| b == b'\n') {
                let line = self.start..self.start + i + 1;
                self.start = line.end;
                return Some(&self.buf[line]);
            }
            self.buf.copy_within(self.start..self.end, 0);
            (self.start, self.end) = (0, self.end - self.start);
            if self.end == BSIZE {
                return None;
            }
            let n = read(&mut self.buf[self.end..]);
            if n <= 0 {
                return None;
            }
            self.end += n as usize;
        }
    }
}

/// factor.c's wheel: the gaps between the numbers coprime to 2*3*5*7 = 210.
/// There are 48, and they sum to 210.
const WHEEL: [f64; 48] = [
    2.0, 10.0, 2.0, 4.0, 2.0, 4.0, 6.0, 2.0, 6.0, 4.0, 2.0, 4.0, 6.0, 6.0, 2.0, 6.0, 4.0, 2.0, 6.0,
    4.0, 6.0, 8.0, 4.0, 2.0, 4.0, 2.0, 4.0, 8.0, 6.0, 4.0, 6.0, 2.0, 4.0, 6.0, 2.0, 6.0, 6.0, 4.0,
    2.0, 4.0, 6.0, 2.0, 6.0, 4.0, 2.0, 4.0, 2.0, 10.0,
];

/// The prime factors of `n`, in the order factor.c prints them: the 2s, 3s,
/// 5s and 7s, then the wheel's candidates, and n itself when it is left over
/// and greater than 1.  factor.c prints n and returns on 0, before dividing;
/// on a negative or infinite n its wheel would loop forever, where sqrt is
/// NaN or d > s never breaks, so the port stops too, with no factors.
pub fn factor(mut n: f64) -> Vec<f64> {
    let mut fs = Vec::new();
    if !n.is_finite() || n <= 0.0 {
        return fs;
    }
    let mut quot = 0.0;
    let mut s = sqrt(n) + 1.0;
    for d in [2.0, 3.0, 5.0, 7.0] {
        while modf(n / d, &mut quot) == 0.0 {
            fs.push(d);
            n = quot;
            s = sqrt(n) + 1.0;
        }
    }
    let mut d = 1.0;
    let mut i = 1;
    loop {
        d += WHEEL[i];
        while modf(n / d, &mut quot) == 0.0 {
            fs.push(d);
            n = quot;
            s = sqrt(n) + 1.0;
        }
        i += 1;
        if i >= WHEEL.len() {
            i = 0;
            if d > s {
                break;
            }
        }
    }
    if n > 1.0 {
        fs.push(n);
    }
    fs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prime() {
        assert_eq!(factor(199.0), vec![199.0], "inside the wheel's first pass");
        assert_eq!(factor(211.0), vec![211.0], "past 209, found by the n > 1");
    }

    #[test]
    fn a_square_of_a_prime() {
        assert_eq!(factor(121.0), vec![11.0, 11.0]);
        assert_eq!(factor(44521.0), vec![211.0, 211.0], "211 squared; the wheel's second pass");
    }

    #[test]
    fn powers_before_the_wheel() {
        assert_eq!(factor(1024.0), vec![2.0; 10]);
        assert_eq!(factor(2187.0), vec![3.0; 7]);
        assert_eq!(factor(78125.0), vec![5.0; 7]);
        assert_eq!(factor(117649.0), vec![7.0; 6]);
    }

    #[test]
    fn one_and_zero() {
        assert_eq!(factor(1.0), Vec::new());
        assert_eq!(factor(0.0), Vec::new(), "factor.c returns on 0");
    }

    #[test]
    fn mixed() {
        assert_eq!(factor(858.0), vec![2.0, 3.0, 11.0, 13.0], "2*3*11*13");
        assert_eq!(factor(422.0), vec![2.0, 211.0], "2*211, the 211 after the wheel breaks");
    }

    #[test]
    fn below_two_fifty_three() {
        // The smoke test's scale: the largest product of two primes below 2 to
        // the 53, exact as a double; a wheel that skipped a candidate would
        // leave the product unbroken.
        assert_eq!(factor(9007195909437503.0), vec![94906247.0, 94906249.0]);
    }

    /// A reader of `chunks`, one each read, then the end of the file.
    fn reader<'a>(chunks: &'a [&'a [u8]]) -> impl FnMut(&mut [u8]) -> isize + 'a {
        let mut chunks = chunks.iter();
        move |buf| match chunks.next() {
            Some(c) => {
                buf[..c.len()].copy_from_slice(c);
                c.len() as isize
            }
            None => 0,
        }
    }

    #[test]
    fn lines_across_reads() {
        let mut read = reader(&[b"10\n2", b"0\n30\n", b"40"]);
        let mut lines = Lines::default();
        assert_eq!(lines.next(&mut read), Some(&b"10\n"[..]));
        assert_eq!(lines.next(&mut read), Some(&b"20\n"[..]), "a line split across reads");
        assert_eq!(lines.next(&mut read), Some(&b"30\n"[..]));
        assert_eq!(lines.next(&mut read), None, "a last line without a newline");
    }

    #[test]
    fn a_console_line_a_read() {
        let mut read = reader(&[b"1001\n", b"7\n"]);
        let mut lines = Lines::default();
        assert_eq!(lines.next(&mut read), Some(&b"1001\n"[..]));
        assert_eq!(lines.next(&mut read), Some(&b"7\n"[..]));
        assert_eq!(lines.next(&mut read), None, "the end of the file");
    }

    #[test]
    fn an_error_or_a_full_buffer_ends_the_lines() {
        let mut lines = Lines::default();
        assert_eq!(lines.next(|_| -1), None, "an error");
        let long = [b'1'; BSIZE];
        let chunks: [&[u8]; 1] = [&long];
        let mut read = reader(&chunks);
        let mut lines = Lines::default();
        assert_eq!(lines.next(&mut read), None, "a full buffer without a newline");
    }
}
