#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::too_long_first_doc_paragraph)]
#![cfg_attr(not(any(test)), no_std)]
#![feature(allocator_api)]
#![feature(maybe_uninit_slice)]
#![feature(step_trait)]
#![feature(unsigned_is_multiple_of)]
#![forbid(unsafe_op_in_unsafe_fn)]

extern crate alloc;

pub mod allocator;
pub mod bitmapalloc;
pub mod dat;
pub mod devcons;
pub mod fdt;
pub mod mcslock;
pub mod mem;
pub mod pagealloc;
