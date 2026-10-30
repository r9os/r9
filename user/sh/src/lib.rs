//! The shell's logic, apart from its system calls, so that it can be tested
//! on the host: splitting a command into words as rc's lexer does, and
//! finding the file to exec as rc's searchpath does.  Neither touches the
//! heap, which never frees, since the shell reads commands until the end of
//! its input.

#![cfg_attr(not(test), no_std)]

use core::ffi::CStr;

/// What parse makes of a command.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// The command's words, this many, NUL-terminated, one after another at
    /// the start of its buffer.  None is a blank line or a comment.
    Words(usize),
    /// The command goes on to the next line: a quote isn't closed, or the
    /// line ends with a backslash.
    More,
    /// The command uses rc syntax that isn't built.
    Unbuilt(u8),
}

/// The bytes that aren't part of an unquoted word, as rc's wordchr, besides
/// the blanks, a newline, a comment's # and a quote: rc's syntax, which
/// isn't built.
fn unbuilt(b: u8) -> bool {
    b";&|^$=`{}()<>".contains(&b)
}

/// Whether `cmd`, which ends with a newline, goes on to the next line, as
/// rc's lexer reads on: a quote that isn't closed, or a backslash before
/// the last newline, outside a quote or comment.
fn more(cmd: &[u8]) -> bool {
    let (mut quoting, mut comment) = (false, false);
    for (i, &b) in cmd.iter().enumerate() {
        match b {
            b'\n' => comment = false,
            _ if comment => {}
            b'\'' => quoting = !quoting,
            b'#' if !quoting => comment = true,
            b'\\' if !quoting && i + 2 == cmd.len() => return true,
            _ => {}
        }
    }
    quoting
}

/// Split `cmd`, one or more lines each ending with a newline, into words, in
/// place, as rc's lexer does: words are separated by blanks; a single-quoted
/// section is part of the word it's in, blanks and newlines included, with
/// a doubled quote inside one standing for a quote; a # outside quotes
/// starts a comment, to the end of its line; and a backslash and newline
/// outside quotes are a blank.  Each word is moved down to the start of the
/// buffer, followed by a NUL, which fits, as the newline at the end is
/// never part of a word.
pub fn parse(cmd: &mut [u8]) -> Parsed {
    if more(cmd) {
        return Parsed::More;
    }
    let (mut r, mut w, mut n) = (0, 0, 0);
    loop {
        while r < cmd.len() && matches!(cmd[r], b' ' | b'\t') {
            r += 1;
        }
        match cmd.get(r) {
            None | Some(b'\n') => return Parsed::Words(n),
            Some(b'#') => {
                r += cmd[r..].iter().position(|&b| b == b'\n').unwrap_or(cmd.len() - r);
                continue;
            }
            Some(b'\\') if cmd.get(r + 1) == Some(&b'\n') => {
                r += 2;
                continue;
            }
            _ => {}
        }
        loop {
            match cmd.get(r) {
                Some(b'\'') => {
                    r += 1;
                    loop {
                        match (cmd[r], cmd.get(r + 1)) {
                            (b'\'', Some(b'\'')) => r += 1,
                            (b'\'', _) => break,
                            _ => {}
                        }
                        cmd[w] = cmd[r];
                        (r, w) = (r + 1, w + 1);
                    }
                    r += 1;
                }
                Some(b' ' | b'\t' | b'\n' | b'#') | None => break,
                Some(b'\\') if cmd.get(r + 1) == Some(&b'\n') => break,
                Some(&b) if unbuilt(b) => return Parsed::Unbuilt(b),
                Some(&b) => {
                    cmd[w] = b;
                    (r, w) = (r + 1, w + 1);
                }
            }
        }
        // The NUL may land on what ended the word, as qtoken's does, so
        // that's read past first.
        if r == w {
            r += match cmd[r] {
                b'#' => cmd[r..].iter().position(|&b| b == b'\n').unwrap_or(cmd.len() - r),
                b'\\' => 2,
                _ => 1,
            };
        }
        cmd[w] = 0;
        w += 1;
        n += 1;
    }
}

/// The words parse left at the start of `cmd`.
pub fn words(cmd: &[u8], n: usize) -> impl Iterator<Item = &CStr> {
    let mut rest = cmd;
    (0..n).map(move |_| {
        let word = CStr::from_bytes_until_nul(rest).expect("parse ends each word with a NUL");
        rest = &rest[word.count_bytes() + 1..];
        word
    })
}

/// The file to exec for the command `word`, written into `buf`, as rc's
/// searchpath and Execute make it: a word starting with /, #, ./ or ../ is
/// used as it is, and any other is looked for in $path, which is /bin.
/// None if it won't fit, where Execute says "command name too long".
pub fn file<'a>(word: &[u8], buf: &'a mut [u8; 1024]) -> Option<&'a CStr> {
    let as_is = [&b"/"[..], b"#", b"./", b"../"].iter().any(|p| word.starts_with(p));
    let dir: &[u8] = if as_is { b"" } else { b"/bin/" };
    let n = dir.len() + word.len();
    if n >= buf.len() {
        return None;
    }
    buf[..dir.len()].copy_from_slice(dir);
    buf[dir.len()..n].copy_from_slice(word);
    buf[n] = 0;
    CStr::from_bytes_with_nul(&buf[..=n]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words of `cmd`, as parse leaves them.
    fn parsed(cmd: &str) -> Result<Vec<String>, Parsed> {
        let mut buf = cmd.as_bytes().to_vec();
        match parse(&mut buf) {
            Parsed::Words(n) => {
                Ok(words(&buf, n).map(|w| w.to_str().unwrap().to_string()).collect())
            }
            other => Err(other),
        }
    }

    #[test]
    fn words_split_on_blanks() {
        assert_eq!(parsed("factor 21\n").unwrap(), ["factor", "21"]);
        assert_eq!(parsed("  echo \t a  b \n").unwrap(), ["echo", "a", "b"], "runs of blanks");
        assert_eq!(parsed("\n").unwrap(), Vec::<String>::new(), "a blank line");
        assert_eq!(parsed(" \t\n").unwrap(), Vec::<String>::new(), "only blanks");
        assert_eq!(parsed("cat #c/cons\n").unwrap(), ["cat"], "# starts a comment");
        assert_eq!(parsed("echo a#b\n").unwrap(), ["echo", "a"], "anywhere outside quotes");
        assert_eq!(parsed("# all comment\n").unwrap(), Vec::<String>::new());
        assert_eq!(parsed("echo a\\b\n").unwrap(), ["echo", "a\\b"], "a backslash in a word");
    }

    #[test]
    fn quotes_are_rcs() {
        assert_eq!(parsed("echo 'a b'\n").unwrap(), ["echo", "a b"]);
        assert_eq!(parsed("cat '#c/cons'\n").unwrap(), ["cat", "#c/cons"], "a quoted #");
        assert_eq!(parsed("echo 'it''s'\n").unwrap(), ["echo", "it's"], "a doubled quote");
        assert_eq!(parsed("echo x'y z'w\n").unwrap(), ["echo", "xy zw"], "joined to the word");
        assert_eq!(parsed("echo ''\n").unwrap(), ["echo", ""], "an empty word");
        assert_eq!(parsed("echo ';'\n").unwrap(), ["echo", ";"], "quoted syntax");
    }

    #[test]
    fn a_command_goes_on() {
        assert_eq!(parsed("echo 'a\n"), Err(Parsed::More), "an open quote");
        assert_eq!(parsed("echo 'a\nb'\n").unwrap(), ["echo", "a\nb"], "a newline in quotes");
        assert_eq!(parsed("echo a\\\n"), Err(Parsed::More), "a backslash at the end");
        assert_eq!(parsed("echo a\\\nb\n").unwrap(), ["echo", "a", "b"], "a blank");
        assert_eq!(parsed("echo a #'\n").unwrap(), ["echo", "a"], "a quote in a comment");
        assert_eq!(parsed("echo a #x\\\n").unwrap(), ["echo", "a"], "a comment doesn't go on");
    }

    #[test]
    fn unbuilt_syntax() {
        for c in [";", "&", "|", "^", "$", "=", "`", "{", "}", "(", ")", "<", ">"] {
            let cmd = format!("echo a{c}b\n");
            assert_eq!(parsed(&cmd), Err(Parsed::Unbuilt(c.as_bytes()[0])), "{c}");
        }
    }

    #[test]
    fn files_are_searched_for_as_rcs() {
        let mut buf = [0; 1024];
        let mut f = |w: &str| file(w.as_bytes(), &mut buf).map(|f| f.to_str().unwrap().to_string());
        assert_eq!(f("factor").as_deref(), Some("/bin/factor"), "in $path");
        assert_eq!(f("/bin/echo").as_deref(), Some("/bin/echo"), "from the root");
        assert_eq!(f("#//boot/echo").as_deref(), Some("#//boot/echo"), "a device's");
        assert_eq!(f("./x").as_deref(), Some("./x"));
        assert_eq!(f("../x").as_deref(), Some("../x"));
        assert_eq!(f(".x").as_deref(), Some("/bin/.x"), "not ./");
        assert_eq!(f(&"x".repeat(1018)).map(|s| s.len()), Some(1023), "the longest that fits");
        assert_eq!(f(&"x".repeat(1019)), None, "too long");
    }
}
