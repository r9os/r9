#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(panic_info_message)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

use port::println;

mod devcons;
mod runtime;
mod sbi;
mod uart16550;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

#[no_mangle]
pub extern "C" fn main9(hartid: usize, opaque: usize) -> ! {
    devcons::init();
    println!();
    println!("r9 from the Internet");
    println!("Domain0 Boot HART = {}, Domain0 Next Arg1 = {:#x}", hartid, opaque);
    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
