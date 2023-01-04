#![allow(clippy::upper_case_acronyms)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![feature(cstr_from_bytes_until_nul)]
#![feature(maybe_uninit_slice)]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod dat;
pub mod devcons;
pub mod fdt;
pub mod mcslock;
