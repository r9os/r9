#![feature(alloc_error_handler)]
#![feature(asm_const)]
#![feature(stdsimd)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod devcons;
mod io;
mod mailbox;
mod registers;
mod trap;
mod uartmini;
mod uartpl011;

use core::ffi::c_void;
use core::ptr;
use port::fdt::DeviceTree;
use port::println;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

fn print_binary_sections() {
    extern "C" {
        static boottext: *const c_void;
        static eboottext: *const c_void;
        static text: *const c_void;
        static etext: *const c_void;
        static rodata: *const c_void;
        static erodata: *const c_void;
        static data: *const c_void;
        static edata: *const c_void;
        static bss: *const c_void;
        static end: *const c_void;
    }

    let bootcode_start: u64 = unsafe { ptr::addr_of!(boottext) as u64 };
    let bootcode_end: u64 = unsafe { ptr::addr_of!(eboottext) as u64 };
    let bootcode_size: u64 = bootcode_end - bootcode_start;

    let text_start: u64 = unsafe { ptr::addr_of!(text) as u64 };
    let text_end: u64 = unsafe { ptr::addr_of!(etext) as u64 };
    let text_size: u64 = text_end - text_start;

    let rodata_start: u64 = unsafe { ptr::addr_of!(rodata) as u64 };
    let rodata_end: u64 = unsafe { ptr::addr_of!(erodata) as u64 };
    let rodata_size: u64 = rodata_end - rodata_start;

    let data_start: u64 = unsafe { ptr::addr_of!(data) as u64 };
    let data_end: u64 = unsafe { ptr::addr_of!(edata) as u64 };
    let data_size: u64 = data_end - data_start;

    let bss_start: u64 = unsafe { ptr::addr_of!(bss) as u64 };
    let bss_end: u64 = unsafe { ptr::addr_of!(end) as u64 };
    let bss_size: u64 = bss_end - bss_start;

    let total_size: u64 = bss_end - bootcode_start;

    println!("Binary sections:");
    println!("  boottext:\t{:#x}-{:#x} ({:#x})", bootcode_start, bootcode_end, bootcode_size);
    println!("  text:\t\t{:#x}-{:#x} ({:#x})", text_start, text_end, text_size);
    println!("  rodata:\t{:#x}-{:#x} ({:#x})", rodata_start, rodata_end, rodata_size);
    println!("  data:\t\t{:#x}-{:#x} ({:#x})", data_start, data_end, data_size);
    println!("  bss:\t\t{:#x}-{:#x} ({:#x})", bss_start, bss_end, bss_size);
    println!("  total:\t{:#x}-{:#x} ({:#x})", bootcode_start, bss_end, total_size);
}

#[no_mangle]
pub extern "C" fn main9(dtb_ptr: u64) {
    trap::init();

    let dt = unsafe { DeviceTree::from_u64(dtb_ptr).unwrap() };
    devcons::init(&dt);

    println!();
    println!("r9 from the Internet");
    println!("DTB found at: {:#x}", dtb_ptr);
    print_binary_sections();

    // Assume we've got MMU set up, so drop early console for the locking console
    port::devcons::drop_early_console();

    println!("looping now");

    #[allow(clippy::empty_loop)]
    loop {}
}

mod runtime;
