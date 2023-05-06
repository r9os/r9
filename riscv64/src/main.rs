#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(panic_info_message)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

extern crate alloc;
pub use alloc::*;

mod address;
mod dat;
mod fns;
mod memory;
mod paging;
mod platform;
mod runtime;
mod sbi;
mod uart16550;

use crate::{
    dat::Mach,
    fns::machp,
    platform::{devcons, platform_init},
};
use port::{fdt::DeviceTree, println};

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

pub static mut MACH: Mach = Mach::new();

#[no_mangle]
pub extern "C" fn main9(hartid: usize, dtb_ptr: usize) -> ! {
    // use sbi for early messaging
    devcons::init_sbi();

    if !machp().is_online() {
        let m = machp();
        m.machno = hartid;
        m.online = true;

        let dt = unsafe { DeviceTree::from_u64(memory::phys_to_virt(dtb_ptr) as u64).unwrap() };
        memory::init_heap(&dt);
        platform_init(&dt);

        println!("switch to UART devcons");
        devcons::init(&dt);

        println!();
        println!("r9 from the Internet");
        println!("Domain0 Boot HART = {hartid}");
        println!("DTB found at: {:x} and {:x}", dtb_ptr, memory::phys_to_virt(dtb_ptr));
        println!();
    } else {
        // startup the other hart's
    }

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
