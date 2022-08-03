#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(asm_sym)]
#![feature(panic_info_message)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

use port::println;

pub const HART_STACK_SIZE: usize = 4 * 4096; // 16KiB
pub const MAX_HART_NUMBER: usize = 8; // QEMU supports upto 8 cores
pub const STACK_SIZE: usize = HART_STACK_SIZE * MAX_HART_NUMBER;

static mut BOOT_STACK: [u8; STACK_SIZE] = [0u8; STACK_SIZE];

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

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"), stack= sym BOOT_STACK, len_per_hart=const HART_STACK_SIZE);

#[macro_use]
mod devcons;
mod runtime;
#[cfg(not(test))]
mod sbi;
mod uart16550;
