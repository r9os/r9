#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod devcons;

use port::fdt::DeviceTree;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

use port::println;

#[no_mangle]
pub extern "C" fn main9(dtb_ptr: u64) {
    let dt = unsafe { DeviceTree::from_u64(dtb_ptr).unwrap() };

    devcons::init(&dt);
    println!();
    println!("r9 from the Internet");
    println!("DTB found at: {:#x}", dtb_ptr);
    println!("looping now");
    #[allow(clippy::empty_loop)]
    loop {}
}

mod runtime;
