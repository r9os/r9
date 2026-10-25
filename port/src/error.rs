//! Error strings, as Plan 9's port/error.h.

pub const EBADARG: &str = "bad arg in system call";
pub const EBADEXEC: &str = "exec header invalid";
pub const EBADFD: &str = "fd out of range or not open";
pub const ENOVMEM: &str = "virtual memory allocation failed";
pub const ESOVERLAP: &str = "segments overlap";
