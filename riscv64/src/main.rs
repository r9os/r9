#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(panic_info_message)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod memory;
mod platform;
mod runtime;
mod sbi;
mod uart16550;

use port::{fdt::Node, print, println};

use crate::{
    memory::phys_to_virt,
    platform::{devcons, platform_init},
};
use core::ptr::{read_volatile, write_volatile};
use core::slice;
use port::fdt::DeviceTree;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

pub fn dump(addr: usize, length: usize) {
    let s = unsafe { slice::from_raw_parts(addr as *const u8, length) };
    println!("dump {length} bytes @{addr:x}");
    for w in s.iter() {
        print!("{:02x}", w);
    }
    println!();
}

pub fn dump_block(base: usize, size: usize, step_size: usize) {
    println!("dump_block {base:x}:{:x}/{step_size:x}", base + size);
    for b in (base..base + size).step_by(step_size) {
        dump(b, step_size);
    }
}

fn read32(a: usize) -> u32 {
    unsafe { core::ptr::read_volatile(a as *mut u32) }
}

fn write32(a: usize, v: u32) {
    unsafe { core::ptr::write_volatile(a as *mut u32, v) }
}

fn consume_dt_block(name: &str, a: u64, l: u64) {
    let v = phys_to_virt(a as usize);
    println!("- {name}: {a:016x}:{v:016x} ({l:x})");
    match name {
        "flash@20000000" => {
            dump_block(v, 0x200, 0x40);
        }
        "test@100000" => {
            let x = read32(v);
            println!("{name}[0]:{x:x}");
            write32(v, x | 0x1234_5678);
            let x = read32(v);
            println!("{name}[0]:{x:x}");
        }
        "pci@30000000" => {
            // NOTE: v+l overflows usize, hitting 0
            // dump_block(v, (l - 0x40) as usize, 0x40);
            dump_block(v, 0x100, 0x40);
        }
        "uart@10000000" => {
            println!("{name}: {l:x}");
            dump_block(v, l as usize, 0x40);
        }
        "virtio_mmio@10001000" | "virtio_mmio@10002000" => {
            dump_block(v, 0x100, 0x40);
        }
        _ => {}
    }
}

const WHERE_ARE_WE: bool = false;

#[no_mangle]
pub extern "C" fn main9(hartid: usize, dtb_ptr: u64) -> ! {
    let dt = unsafe { DeviceTree::from_u64(dtb_ptr).unwrap() };

    devcons::init(&dt);
    println!("\n--> DT / native devcons\n");
    devcons::init_sbi();
    println!("\n--> SBI devcons\n");
    println!("dtb@{dtb_ptr:x}");

    platform_init();

    println!("r9 from the Internet");
    println!("Domain0 Boot HART = {hartid}");
    println!("DTB found at: {dtb_ptr:#x}");

    dt.nodes().for_each(|n| {
        let name = dt.node_name(&n);
        if false {
            if let Some(name) = name {
                let p = dt.property(&n, "start");
                println!("{name:?}: {n:#?} {p:?}");
            }
        }
        dt.property_translated_reg_iter(n).next().and_then(|i| {
            let b = i.regblock();
            if let Some(b) = b {
                // println!("{b:#?}");
                let a = b.addr;
                if let Some(name) = name {
                    if let Some(l) = b.len {
                        consume_dt_block(name, a, l);
                    }
                }
            }
            b
        });
    });

    // check on memory mapping foo
    if WHERE_ARE_WE {
        let x = "test";
        let p = x.as_ptr();
        // e.g., 0xffffffffc0400096
        println!("{p:#x?}");
    }

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
