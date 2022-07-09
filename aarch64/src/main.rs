#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(asm_sym)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod devcons;

core::arch::global_asm!(include_str!("l.S"));

use port::println;

#[no_mangle]
pub extern "C" fn main9() {
    devcons::init();
    println!();
    println!("r9 from the Internet");
    println!("looping now");
    #[allow(clippy::empty_loop)]
    loop {}
}

mod runtime;
