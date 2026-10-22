#![allow(clippy::too_many_arguments)]
#![allow(clippy::upper_case_acronyms)]
#![allow(internal_features)]
#![cfg_attr(not(any(test)), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), feature(alloc_error_handler))]
#![feature(core_intrinsics)]
#![feature(sync_unsafe_cell)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod allocator;
mod devcons;
mod deviceutil;
mod io;
mod kmem;
mod mailbox;
mod pagealloc;
mod param;
mod pre_mmu;
mod reg;
mod registers;
#[allow(dead_code)] // Unused until processes need kernel context switches
mod swtch;
mod trap;
mod uartmini;
mod uartpl011;
mod vm;
mod vmdebug;

extern crate alloc;

use alloc::boxed::Box;
use param::KZERO;
use port::fdt::DeviceTree;
use port::mem::{PAGE_SIZE_4K, PhysAddr, PhysRange, VirtRange};
use port::println;
use reg::midr_el1::MidrEl1;
use vm::{Entry, RootPageTableType, VaMapping};

use crate::kmem::{
    boottext_physrange, bss_physrange, data_physrange, rodata_physrange, text_physrange,
    total_kernel_physrange,
};
use crate::vm::PageSize;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

fn print_memory_range(name: &str, range: &PhysRange) {
    let size = range.size();
    println!("  {name}{range} ({size:#x})");
}

fn print_binary_sections() {
    println!("Binary sections:");
    print_memory_range("boottext:\t", &boottext_physrange());
    print_memory_range("text:\t\t", &text_physrange());
    print_memory_range("rodata:\t", &rodata_physrange());
    print_memory_range("data:\t\t", &data_physrange());
    print_memory_range("bss:\t\t", &bss_physrange());
    print_memory_range("total:\t", &total_kernel_physrange());
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

fn print_stacks() {
    unsafe extern "C" {
        static interruptstackbase: [u64; 0];
        static interruptstacksz: [u64; 0];
    }

    let interrupt_stack_base = unsafe { interruptstackbase.as_ptr().addr() };
    let interrupt_stack_max = interrupt_stack_base + unsafe { interruptstacksz.as_ptr().addr() };
    let range = VirtRange::new(interrupt_stack_base, interrupt_stack_max);
    let range_size = range.size();
    println!("Interrupt stack:\t {range} ({range_size:#x})");
}

/// dtb_va is the virtual address of the DTB structure.  The physical address is
/// assumed to be dtb_va-KZERO.
#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    trap::init();

    // Parse the DTB before we set up memory so we can correctly map it
    let dt = unsafe { DeviceTree::from_usize(dtb_va).unwrap() };
    let dtb_physrange = PhysRange::with_pa_len(PhysAddr::new((dtb_va - KZERO) as u64), dt.size());

    // Set up page allocator
    let mut physranges = [
        dtb_physrange.round(PageSize::Page4K.size()),
        boottext_physrange().add(&text_physrange()),
        rodata_physrange(),
        data_physrange().add(&bss_physrange()),
    ];
    physranges.sort_by_key(|a| a.start);
    if let Err(err) = pagealloc::init_page_allocator(&dt, physranges.into_iter()) {
        panic!("error:Couldn't mark unused pages as free: err: {:?}", err);
    }

    devcons::init(&dt);
    mailbox::init(&dt);

    println!();
    println!("r9 from the Internet");
    println!("DTB found at: {:#x}", dtb_va);
    println!("midr_el1: {:?}", MidrEl1::read());

    print_stacks();

    print_binary_sections();

    print_board_info();
    print_memory_info();

    // vmdebug::print_recursive_tables(RootPageTableType::Kernel);
    // vmdebug::print_recursive_tables(RootPageTableType::User);

    {
        let page_table = vm::kernel_pagetable();
        let entry = Entry::rw_kernel_data();
        for i in 0..3 {
            let alloc_result = pagealloc::allocate_virtpage(
                page_table,
                "testkernel",
                entry,
                VaMapping::Offset(KZERO),
                RootPageTableType::Kernel,
            );
            match alloc_result {
                Ok(_allocated_page) => {}
                Err(err) => {
                    println!("Error allocating page in kernel space ({i}): {:?}", err);
                    break;
                }
            }
        }
    }

    // vmdebug::print_recursive_tables(RootPageTableType::Kernel);
    // vmdebug::print_recursive_tables(RootPageTableType::User);

    println!("Set up a user process");

    let _b = Box::new("ddododo");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    test_user_syscalls();
}

mod runtime;

fn test_user_syscalls() -> ! {
    // Temporary locations until the user address space layout is defined.
    const USER_TEXT_VA: usize = 0x1000;
    const USER_STACK_VA: usize = 0x2000;

    let page_table = vm::user_pagetable();

    // Allocate pages for a user process
    let user_text = pagealloc::allocate_virtpage(
        page_table,
        "usertext",
        Entry::rw_user_text(),
        VaMapping::Addr(USER_TEXT_VA),
        RootPageTableType::User,
    )
    .expect("couldn't allocate user_text");

    // Machine code and assembly to check a syscall returns to EL0, then exit
    //   20 05 80 D2    ; mov x0, #41
    //   01 00 00 D4    ; svc #0        (returns x0 + 1)
    //   61 00 00 D4    ; svc #3        (exit)
    let proc_text_bytes: [u8; 12] =
        [0x20, 0x05, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x61, 0x00, 0x00, 0xd4];
    user_text.0[..proc_text_bytes.len()].copy_from_slice(&proc_text_bytes);

    pagealloc::allocate_virtpage(
        page_table,
        "userstack",
        Entry::rw_user_data(),
        VaMapping::Addr(USER_STACK_VA),
        RootPageTableType::User,
    )
    .expect("couldn't allocate user_stack");

    // Exceptions from EL0 are taken on the interrupt stack for now.
    unsafe extern "C" {
        static interruptstackbase: [u64; 0];
        static interruptstacksz: [u64; 0];
    }
    let kstack_top =
        unsafe { interruptstackbase.as_ptr().addr() + interruptstacksz.as_ptr().addr() };

    vmdebug::print_recursive_tables(RootPageTableType::Kernel);
    vmdebug::print_recursive_tables(RootPageTableType::User);

    println!("Executing user process");
    unsafe { trap::touser(kstack_top, USER_TEXT_VA as u64, (USER_STACK_VA + PAGE_SIZE_4K) as u64) }
}
