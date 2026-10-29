//! Error strings, as Plan 9's port/error.h.

pub const EBADARG: &str = "bad arg in system call";
pub const EBADFD: &str = "fd out of range or not open";
pub const EBADSHARP: &str = "unknown device in # filename";
pub const EBADUSEFD: &str = "inappropriate use of fd";
pub const EBADEXEC: &str = "exec header invalid";
pub const EISDIR: &str = "file is a directory";
pub const ENAMETOOLONG: &str = "name too long";
pub const ENEGOFF: &str = "negative i/o offset";
pub const ENOFD: &str = "no free file descriptors";
pub const ENONEXIST: &str = "file does not exist";
pub const ENOTSUP: &str = "not supported";
pub const ENOTDIR: &str = "not a directory";
pub const ENOVMEM: &str = "virtual memory allocation failed";
pub const EPERM: &str = "permission denied";
pub const ESOVERLAP: &str = "segments overlap";
