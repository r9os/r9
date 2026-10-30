//! A shell that runs simple commands as Plan 9's rc does: it prompts "% ",
//! reads a command, splits it into words with rc's quotes and comments,
//! finds the program as rc's searchpath does, and forks, execs and waits.
//! A failed exec prints the word and the error, as rc's Execute, and a
//! command's failing status prints nothing, as in rc.  At the end of its
//! input, it exits with the last command's status.  rc's other syntax isn't
//! built, so it isn't called rc.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use libc::{
    ERRMAX, Errstr, Lines, exec, exits, fork, fprint, read, rerrstr, wait, werrstr, write,
};
use sh::{Parsed, file, parse, words};

/// The longest command, its lines together, as a Biobuf's.
const CMDSIZE: usize = 8 * 1024;

/// Run the command whose `n` words parse left in `cmd`, as rc's Xsimple:
/// fork, then in the child exec it, and in the parent wait for it, as rc's
/// Waitfor, setting `status` to its exit's.  Return the status's length.
/// Failing to fork, it's rc's "try again".
fn run(cmd: &[u8], n: usize, status: &mut [u8]) -> usize {
    let pid = fork();
    if pid < 0 {
        fprint(2, format_args!("sh: try again: {Errstr}\n"));
        return copy(status, b"try again");
    }
    if pid == 0 {
        execute(cmd, n);
    }
    while let Some(w) = wait() {
        if w.pid == pid {
            return copy(status, w.msg());
        }
    }
    0
}

/// Exec the command, in the child, as rc's Execute, then, if that fails, say
/// why and exit, as rc's execforkexec does.  The heap the words' array uses
/// is the child's, so it goes when the child does.
fn execute(cmd: &[u8], n: usize) -> ! {
    let argv: Vec<_> = words(cmd, n).collect();
    let word = argv[0].to_bytes();
    let mut buf = [0; 1024];
    match file(word, &mut buf) {
        Some(f) => {
            exec(f, &argv);
        }
        None => werrstr("command name too long"),
    }
    let mut err = [0; ERRMAX];
    rerrstr(&mut err);
    let err = &err[..err.iter().position(|&b| b == 0).unwrap_or(0)];
    for s in [word, b": ", err, b"\n"] {
        write(2, s);
    }
    let mut msg = [0; ERRMAX];
    let n = copy(&mut msg, b"can't exec: ");
    let n = n + copy(&mut msg[n..], err);
    exits(core::str::from_utf8(&msg[..n]).unwrap_or("can't exec"))
}

/// Copy as much of `src` as fits into `dst`, and return how much.
fn copy(dst: &mut [u8], src: &[u8]) -> usize {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    n
}

#[unsafe(no_mangle)]
extern "C" fn main(_argc: usize, _argv: *const *const u8) {
    let mut lines = Lines::default();
    let mut cmd = [0u8; CMDSIZE];
    let mut status = [0u8; ERRMAX];
    let mut slen = 0;
    let mut len = 0;
    loop {
        write(2, if len == 0 { b"% " } else { b"\t" });
        let Some(line) = lines.next(|buf| read(0, buf)) else {
            exits(core::str::from_utf8(&status[..slen]).unwrap_or("sh"));
        };
        if len + line.len() > CMDSIZE {
            fprint(2, format_args!("sh: command too long\n"));
            len = 0;
            continue;
        }
        cmd[len..len + line.len()].copy_from_slice(line);
        len += line.len();
        match parse(&mut cmd[..len]) {
            Parsed::More => continue,
            Parsed::Words(0) => {}
            Parsed::Words(n) => slen = run(&cmd[..len], n, &mut status),
            Parsed::Unbuilt(c) => {
                fprint(2, format_args!("sh: rc's {} isn't built\n", c as char));
            }
        }
        len = 0;
    }
}
