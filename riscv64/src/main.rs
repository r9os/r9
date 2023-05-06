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
mod memory;
mod paging;
mod platform;
mod runtime;
mod sbi;
mod uart16550;

use crate::{
    dat::Mach,
    platform::{devcons, platform_init},
};
use port::{fdt::DeviceTree, println};

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

pub static mut MACH: *mut Mach = core::ptr::null_mut();

/// get a reference to the boot Mach
pub fn machp() -> &'static mut Mach {
    unsafe { &mut (*MACH) }
}

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
    // use sbi for early messaging
    devcons::init_sbi();

    if unsafe { MACH.is_null() } {
        println!("setting boot Mach pointer");
        // set the pointer to the first Mach
        // currently the heap start which is wrong! but ok for now
        unsafe { MACH = &mut *(&memory::end as *const usize as *mut Mach) };

        // get Mach and initialize with the boot hartid
        let m = machp();
        println!("init the first Mach");
        *m = Mach::new();

        m.machno = hartid;
        m.online = 1;

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

    // list_dtb(&dt);

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
