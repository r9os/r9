#![allow(clippy::upper_case_acronyms)]
#![cfg_attr(not(any(test)), no_std)]
#![feature(maybe_uninit_slice)]
#![feature(step_trait)]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod bitmapalloc;
pub mod dat;
pub mod devcons;
pub mod fdt;
pub mod mcslock;
pub mod mem;
pub mod vmem;
