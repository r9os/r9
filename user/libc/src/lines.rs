//! Reading lines, after Plan 9's libbio: Brdline's reading through a
//! buffer, for a program reading its input a line at a time.

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

#[cfg(test)]
mod tests {
    use super::*;

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
