//! What the kernel and user programs must agree on, as Plan 9's
//! libc/9syscall/sys.h: the system call numbers, and the longest error or exit
//! string.

#![no_std]

pub const SYSR1: u64 = 0;
pub const CLOSE: u64 = 4;
pub const DUP: u64 = 5;
pub const EXITS: u64 = 8;
pub const OPEN: u64 = 14;
pub const SLEEP: u64 = 17;
pub const BRK_: u64 = 24;
pub const SEEK: u64 = 39;
pub const ERRSTR: u64 = 41;
pub const PREAD: u64 = 50;
pub const PWRITE: u64 = 51;

/// The longest error or exit string, with its NUL, as Plan 9's ERRMAX.
pub const ERRMAX: usize = 128;
