#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::too_long_first_doc_paragraph)]
#![cfg_attr(not(any(test)), no_std)]
#![feature(step_trait)]
#![forbid(unsafe_op_in_unsafe_fn)]

extern crate alloc;

pub mod allocator;
pub mod bitmapalloc;
pub mod chan;
pub mod clock;
pub mod console;
pub mod dat;
pub mod dev;
pub mod devcons;
pub mod devroot;
pub mod elf;
pub mod error;
pub mod exec;
#[cfg(test)]
mod fake;
pub mod fdt;
pub mod fixedstring;
pub mod irq;
pub mod maths;
pub mod mcslock;
pub mod mem;
pub mod pagealloc;
pub mod proc;
pub mod segment;
pub mod syscall;
pub mod timer;
pub mod uaccess;

pub type Result<T> = core::result::Result<T, &'static str>;
