//! A child's exit, as Plan 9's wait parses await's text into a Waitmsg, with
//! tokenize, which splits it into fields as rc quotes them.  Neither touches
//! the heap, which never frees, since a shell waits once a command.

use sys::ERRMAX;

use crate::parse::strtol;

/// A child's exit, as Plan 9's Waitmsg: its pid, its user, system and real
/// times in milliseconds, and its exit status, empty for success, held in
/// the Waitmsg, as a status is at most ERRMAX bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Waitmsg {
    pub pid: i32,
    pub time: [u32; 3],
    msg: [u8; ERRMAX],
    len: usize,
}

impl Waitmsg {
    /// The exit status, empty for success.
    pub fn msg(&self) -> &[u8] {
        &self.msg[..self.len]
    }
}

/// The separators tokenize splits on, as Plan 9's qsep.
fn qsep(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// Split `s` into at most N fields, in place, as Plan 9's tokenize: separated
/// by runs of blanks, which a single-quoted section keeps, with a doubled
/// quote inside one standing for a quote.  The quotes are removed by moving
/// what follows them down, as qtoken does, so each field is a slice of `s`.
/// Return the fields, and how many there are; the rest are empty.
pub fn tokenize<const N: usize>(s: &mut [u8]) -> ([&[u8]; N], usize) {
    let mut ranges = [(0, 0); N];
    let mut n = 0;
    let mut t = 0;
    while n < N {
        while t < s.len() && qsep(s[t]) {
            t += 1;
        }
        if t == s.len() {
            break;
        }
        // The field is written from where it starts, w, as it's read from t.
        let start = t;
        let mut w = t;
        let mut quoting = false;
        while t < s.len() && (quoting || !qsep(s[t])) {
            if s[t] != b'\'' {
                s[w] = s[t];
                w += 1;
            } else if !quoting {
                quoting = true;
            } else if s.get(t + 1) != Some(&b'\'') {
                quoting = false;
            } else {
                t += 1;
                s[w] = b'\'';
                w += 1;
            }
            t += 1;
        }
        ranges[n] = (start, w);
        n += 1;
    }
    let s: &[u8] = s;
    (core::array::from_fn(|i| &s[ranges[i].0..ranges[i].1]), n)
}

/// The Waitmsg await's text `text` holds, "pid utime stime rtime msg", as
/// Plan 9's wait parses it, tokenizing `text` in place: None unless it has
/// five fields.  Each number is atoi's, the longest decimal prefix of its
/// field.
pub fn parse_waitmsg(text: &mut [u8]) -> Option<Waitmsg> {
    let ([pid, t0, t1, t2, msg], 5) = tokenize::<5>(text) else {
        return None;
    };
    let atoi = |f: &[u8]| strtol(f, 10).0;
    let len = msg.len().min(ERRMAX);
    let mut w = Waitmsg {
        pid: atoi(pid) as i32,
        time: [atoi(t0) as u32, atoi(t1) as u32, atoi(t2) as u32],
        msg: [0; ERRMAX],
        len,
    };
    w.msg[..len].copy_from_slice(&msg[..len]);
    Some(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;

    fn tokens<const N: usize>(s: &str) -> Vec<String> {
        let mut buf = s.as_bytes().to_vec();
        let (fields, n) = tokenize::<N>(&mut buf);
        fields[..n].iter().map(|f| String::from_utf8(f.to_vec()).unwrap()).collect()
    }

    #[test]
    fn tokenizes_as_plan9() {
        assert_eq!(tokens::<5>("  a b\tc\n"), ["a", "b", "c"], "blanks");
        assert_eq!(tokens::<5>("a 'b c' d"), ["a", "b c", "d"], "a quoted section");
        assert_eq!(tokens::<5>("'it''s'"), ["it's"], "a doubled quote");
        assert_eq!(tokens::<5>("x'y z'w"), ["xy zw"], "quotes inside a field");
        assert_eq!(tokens::<5>("''"), [""], "an empty quoted field");
        assert_eq!(tokens::<5>("'open"), ["open"], "an unclosed quote runs to the end");
        assert_eq!(tokens::<2>("a b c"), ["a", "b"], "at most N");
        assert_eq!(tokens::<5>(""), Vec::<String>::new());
    }

    fn parse(s: &[u8]) -> Option<(i32, [u32; 3], Vec<u8>)> {
        parse_waitmsg(&mut s.to_vec()).map(|w| (w.pid, w.time, w.msg().to_vec()))
    }

    #[test]
    fn parses_await_text() {
        assert_eq!(parse(b"12 0 0 0 ''"), Some((12, [0, 0, 0], vec![])), "a success");
        let failure = parse(b"7 1 2 3 'cat 7: can''t open x'");
        assert_eq!(failure, Some((7, [1, 2, 3], b"cat 7: can't open x".to_vec())));
        assert_eq!(parse(b"7 0 0 0 plain").map(|w| w.2), Some(b"plain".to_vec()));
        assert_eq!(parse(b"7 0 0 0 '\xff'").map(|w| w.2), Some(vec![0xff]), "bytes as they are");
        assert_eq!(parse(b"7 0 0 0"), None, "four fields");
        assert_eq!(parse(b""), None, "nothing");
    }
}
