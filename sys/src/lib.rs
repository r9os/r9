//! What the kernel and user programs must agree on, as Plan 9's
//! libc/9syscall/sys.h: the system call numbers, the longest error or exit
//! string, and, as Plan 9's libc.h, the open modes.

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
pub const AWAIT: u64 = 47;
pub const PREAD: u64 = 50;
pub const PWRITE: u64 = 51;

/// The longest error or exit string, with its NUL, as Plan 9's ERRMAX.
pub const ERRMAX: usize = 128;

/// The open modes, as Plan 9's libc.h: an access, and the flags or'ed in.
pub const OREAD: u32 = 0;
pub const OWRITE: u32 = 1;
pub const ORDWR: u32 = 2;
pub const OEXEC: u32 = 3;
pub const OTRUNC: u32 = 16;
pub const OCEXEC: u32 = 32;
pub const ORCLOSE: u32 = 64;
