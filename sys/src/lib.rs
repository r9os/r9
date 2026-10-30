//! What the kernel and user programs must agree on, as Plan 9's
//! libc/9syscall/sys.h: the system call numbers, the longest error or exit
//! string, and, as Plan 9's libc.h, the open modes and rfork's flags.

#![no_std]

pub const SYSR1: u64 = 0;
pub const CLOSE: u64 = 4;
pub const DUP: u64 = 5;
pub const EXEC: u64 = 7;
pub const EXITS: u64 = 8;
pub const OPEN: u64 = 14;
pub const SLEEP: u64 = 17;
pub const RFORK: u64 = 19;
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

/// The rfork flags, as Plan 9's libc.h.
pub const RFNAMEG: u32 = 1 << 0;
pub const RFENVG: u32 = 1 << 1;
pub const RFFDG: u32 = 1 << 2;
pub const RFNOTEG: u32 = 1 << 3;
pub const RFPROC: u32 = 1 << 4;
pub const RFMEM: u32 = 1 << 5;
pub const RFNOWAIT: u32 = 1 << 6;
pub const RFCNAMEG: u32 = 1 << 10;
pub const RFCENVG: u32 = 1 << 11;
pub const RFCFDG: u32 = 1 << 12;
pub const RFREND: u32 = 1 << 13;
pub const RFNOMNT: u32 = 1 << 14;
