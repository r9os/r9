//! The user library, after Plan 9's libc: the program's entry, system calls,
//! a heap and formatted output, and the number parsing and math the commands
//! use.  A program is a `no_main` binary that defines
//! `#[unsafe(no_mangle)] extern "C" fn main(argc: usize, argv: *const *const
//! u8)`, and calls exits.
//!
//! The parsing and math are pure, so they're tested on the host.  The rest is
//! only for r9, so on any other target it's absent: a crate that uses this
//! one can still build its library, and test it, on the host.

#![cfg_attr(not(test), no_std)]

mod math;
mod parse;
#[cfg(target_os = "none")]
mod runtime;

pub use math::{modf, sqrt};
pub use parse::{atof, strtol};
#[cfg(target_os = "none")]
pub use runtime::{Errstr, brk, brk_, errstr, exits, fprint, pwrite, rerrstr, sbrk, sleep, write};
