#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(panic_info_message)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod board;
mod runtime;
mod sbi;
mod uart16550;

use port::fdt::DeviceTree;
use port::println;

use crate::board::devcons;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

#[no_mangle]
pub extern "C" fn main9(hartid: usize, dtb_ptr: u64) -> ! {
    let dt = unsafe { DeviceTree::from_u64(dtb_ptr).unwrap() };

    devcons::init(&dt);
    println!();
    println!("r9 from the Internet");
    println!("Domain0 Boot HART = {hartid}");
    println!("DTB found at: {dtb_ptr:#x}");

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
