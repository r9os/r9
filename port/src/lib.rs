#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod devcons;
pub mod mcslock;
