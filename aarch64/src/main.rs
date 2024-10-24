#![allow(clippy::upper_case_acronyms)]
#![allow(internal_features)]
#![cfg_attr(not(any(test)), no_std)]
#![cfg_attr(not(test), no_main)]
#![feature(alloc_error_handler)]
#![feature(core_intrinsics)]
#![feature(sync_unsafe_cell)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod devcons;
mod io;
mod kmem;
mod mailbox;
mod pagealloc;
mod param;
mod registers;
mod trap;
mod uartmini;
mod uartpl011;
mod vm;

use crate::kmem::from_virt_to_physaddr;
use crate::vm::kernel_root;
use core::ptr;
use kmem::{boottext_range, bss_range, data_range, rodata_range, text_range, total_kernel_range};
use port::fdt::DeviceTree;
use port::mem::PhysRange;
use port::println;
use vm::PageTable;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

static mut KPGTBL: PageTable = PageTable::empty();

unsafe fn print_memory_range(name: &str, range: &PhysRange) {
    let size = range.size();
    println!("  {name}{range} ({size:#x})");
}

fn print_binary_sections() {
    println!("Binary sections:");
    unsafe {
        print_memory_range("boottext:\t", &boottext_range());
        print_memory_range("text:\t\t", &text_range());
        print_memory_range("rodata:\t", &rodata_range());
        print_memory_range("data:\t\t", &data_range());
        print_memory_range("bss:\t\t", &bss_range());
        print_memory_range("total:\t", &total_kernel_range());
    }
}

fn print_physical_memory_info() {
    println!("Physical memory map:");
    let arm_mem = mailbox::get_arm_memory();
    println!("  Memory:\t{arm_mem} ({:#x})", arm_mem.size());
    let vc_mem = mailbox::get_vc_memory();
    println!("  Video:\t{vc_mem} ({:#x})", vc_mem.size());
}

fn print_memory_info() {
    println!("Memory usage:");
    let (used, total) = pagealloc::usage_bytes();
    println!("  Used:\t\t{used:#016x}");
    println!("  Total:\t{total:#016x}");
}

// https://github.com/raspberrypi/documentation/blob/develop/documentation/asciidoc/computers/raspberry-pi/revision-codes.adoc
fn print_pi_name(board_revision: u32) {
    let name = match board_revision {
        0xa21041 => "Raspberry Pi 2B",
        0xa02082 => "Raspberry Pi 3B",
        0xb03115 => "Raspberry Pi 4B",
        0xa220a0 => "Raspberry Compute Module 3",
        _ => "Unrecognised",
    };
    println!("  Board Name:\t{name}");
}

fn print_board_info() {
    println!("Board information:");
    let board_revision = mailbox::get_board_revision();
    print_pi_name(board_revision);
    println!("  Board Rev:\t{board_revision:#010x}");
    let model = mailbox::get_board_model();
    println!("  Board Model:\t{model:#010x}");
    let serial = mailbox::get_board_serial();
    println!("  Serial Num:\t{serial:#010x}");
    let mailbox::MacAddress { a, b, c, d, e, f } = mailbox::get_board_macaddr();
    println!("  MAC Address:\t{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}");
    let fw_revision = mailbox::get_firmware_revision();
    println!("  Firmware Rev:\t{fw_revision:#010x}");
}

/// dtb_va is the virtual address of the DTB structure.  The physical address is
/// assumed to be dtb_va-KZERO.
#[no_mangle]
pub extern "C" fn main9(dtb_va: usize) {
    trap::init();

    // Parse the DTB before we set up memory so we can correctly map it
    let dt = unsafe { DeviceTree::from_usize(dtb_va).unwrap() };

    // Set up uart so we can log as early as possible
    mailbox::init(&dt);
    devcons::init(&dt);

    println!();
    println!("r9 from the Internet");
    println!("DTB found at: {:#x}", dtb_va);
    println!("midr_el1: {:?}", registers::MidrEl1::read());

    print_binary_sections();
    print_physical_memory_info();
    print_board_info();

    // Map address space accurately using rust VM code to manage page tables
    unsafe {
        let dtb_range = PhysRange::with_len(from_virt_to_physaddr(dtb_va).addr(), dt.size());
        vm::init(&mut *ptr::addr_of_mut!(KPGTBL), dtb_range, mailbox::get_arm_memory());
        vm::switch(&*ptr::addr_of!(KPGTBL));
    }

    // From this point we can use the global allocator

    print_memory_info();

    if let Ok(page) = pagealloc::allocate() {
        println!("page addr: {:#016x}", page.data().as_ptr() as *const _ as u64);

        //let mapped_range =
        // let kpgtable = unsafe { &mut *ptr::addr_of_mut!(KPGTBL) };
        // kpgtable.map_phys_range(range, *flags, *page_size).expect("dynamic mapping failed");
    }

    kernel_root().print_recursive_tables();

    println!("looping now");

    #[allow(clippy::empty_loop)]
    loop {}
}
mod runtime;
