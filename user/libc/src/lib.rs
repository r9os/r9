//! The user library, after Plan 9's libc: the program's entry, system calls,
//! a heap and formatted output, and the number parsing, math, line reading
//! and wait messages the commands use.  A program is a `no_main` binary that defines
//! `#[unsafe(no_mangle)] extern "C" fn main(argc: usize, argv: *const *const
//! u8)`, and calls exits.
//!
//! The parsing, math, line reading and wait messages are pure, so they're
//! tested on the host.  The rest is
//! only for r9, so on any other target it's absent: a crate that uses this
//! one can still build its library, and test it, on the host.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod lines;
mod math;
mod parse;
#[cfg(target_os = "none")]
mod runtime;
mod wait;

pub use lines::Lines;
pub use math::{modf, sqrt};
pub use parse::{atof, strtol};
#[cfg(target_os = "none")]
pub use runtime::{
    ERRMAX, Errstr, OCEXEC, OEXEC, OREAD, ORCLOSE, ORDWR, OTRUNC, OWRITE, RFCENVG, RFCFDG, RFCNAMEG,
    RFENVG, RFFDG, RFMEM, RFNAMEG, RFNOMNT, RFNOTEG, RFNOWAIT, RFPROC, RFREND, r#await, brk, brk_,
    close, dup, errstr, exec, exits, fork, fprint, open, pread, pwrite, read, rerrstr, rfork, sbrk,
    seek, sleep, wait, werrstr, write,
};
pub use wait::{Waitmsg, parse_waitmsg, tokenize};
