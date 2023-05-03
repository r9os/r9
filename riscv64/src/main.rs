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

use crate::{
    paging::PageTable,
    platform::{devcons, platform_init},
};
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
    let dt = unsafe { DeviceTree::from_u64(memory::phys_to_virt(dtb_ptr) as u64).unwrap() };

    // use sbi for early messages
    devcons::init_sbi();

    extern "C" {
        pub(crate) static mut boot_page_table: PageTable;
    }
    println!();
    let root = unsafe { &mut boot_page_table };

    if let Some(entry) = paging::virt_to_phys(root, 0xFFFF_FFFF_C000_0000) {
        println!("0xFFFF_FFFF_C000_0000 => 0x{:X}", entry);
    } else {
        println!("0xFFFF_FFFF_C000_0000 not found");
    }

    if let Some(entry) = paging::virt_to_phys(root, 0x8000_0000) {
        println!("0x8000_0000 => 0x{:X}", entry);
    } else {
        println!("0x8000_0000 not found");
    }

    // map the UART port
    // map uses an hack to work, don't use this for something else!!
    paging::map(
        root,
        memory::phys_to_virt(0x1000_0000),
        0x1000_0000,
        paging::EntryBits::ReadWrite.val(),
        0, // level 0 = 4k page
    );

    if let Some(entry) = paging::virt_to_phys(root, memory::phys_to_virt(0x1000_0000)) {
        println!("0x{:X} => 0x{:X}", memory::phys_to_virt(0x1000_0000), entry);
    } else {
        println!("0x{:X} not found", memory::phys_to_virt(0x1000_0000));
    }

    // switch to UART
    devcons::init(&dt);
    platform_init();
    println!();
    println!("r9 from the Internet");
    println!("Domain0 Boot HART = {hartid}");
    println!("DTB found at: {:x}", memory::phys_to_virt(dtb_ptr));
    println!();

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
