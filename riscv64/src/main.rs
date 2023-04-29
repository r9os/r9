#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(panic_info_message)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod address;
mod memory;
mod paging;
mod platform;
mod runtime;
mod sbi;
mod uart16550;

use port::println;

use crate::platform::{devcons, platform_init};
use port::fdt::DeviceTree;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

fn list_dtb(dt: &DeviceTree) {
    for n in dt.nodes() {
        if let Some(name) = DeviceTree::node_name(&dt, &n) {
            let reg_block_iter = DeviceTree::property_reg_iter(&dt, n);
            for b in reg_block_iter {
                println!("{} at 0x{:x} len {:?}", name, b.addr, b.len);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn main9(hartid: usize, dtb_ptr: usize) -> ! {
    let dt = unsafe { DeviceTree::from_u64(dtb_ptr as u64).unwrap() };

    // devcons::init(&dt);
    devcons::init_sbi();
    platform_init();

    println!();
    println!("r9 from the Internet");
    println!("Domain0 Boot HART = {hartid}");
    println!("DTB found at: {dtb_ptr:#x}");
    println!();

    list_dtb(&dt);

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
