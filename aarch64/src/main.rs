#![allow(clippy::upper_case_acronyms)]
#![allow(internal_features)]
#![cfg_attr(not(any(test)), no_std)]
#![cfg_attr(not(test), no_main)]
#![feature(alloc_error_handler)]
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
mod registers;
mod swtch;
mod trap;
mod uartmini;
mod uartpl011;
mod vm;
mod vmdebug;

extern crate alloc;

use alloc::boxed::Box;
use core::ptr::null_mut;
use kmem::{boottext_range, bss_range, data_range, rodata_range, text_range, total_kernel_range};
use param::KZERO;
use port::mem::PhysRange;
use port::println;
use port::{fdt::DeviceTree, mem::PhysAddr};
use vm::{Entry, RootPageTableType, VaMapping};

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

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
    println!("Interrupt stack: {:018x}..{:018x}", interrupt_stack_base, interrupt_stack_max);
}

/// dtb_va is the virtual address of the DTB structure.  The physical address is
/// assumed to be dtb_va-KZERO.
#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    trap::init();

    // Parse the DTB before we set up memory so we can correctly map it
    let dt = unsafe { DeviceTree::from_usize(dtb_va).unwrap() };
    let dtb_physrange = PhysRange::with_pa_len(PhysAddr::new((dtb_va - KZERO) as u64), dt.size());

    // Try to set up the miniuart so we can log as early as possible.
    devcons::init(&dt, true);

    println!();
    println!("r9 from the Internet");
    println!("DTB found at: {:#x}", dtb_va);
    println!("midr_el1: {:?}", registers::MidrEl1::read());

    print_stacks();

    print_binary_sections();

    pagealloc::init_page_allocator();

    // Map address space accurately using rust VM code to manage page tables
    unsafe {
        vm::init_kernel_page_tables(&dt, dtb_physrange);
        vm::switch(vm::kernel_pagetable(), RootPageTableType::Kernel);

        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // From this point we can use the global allocator

    devcons::init(&dt, false);
    mailbox::init(&dt);

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

    test_sysexit();

    vmdebug::print_recursive_tables(RootPageTableType::Kernel);
    vmdebug::print_recursive_tables(RootPageTableType::User);

    let _b = Box::new("ddododo");

    println!("looping now");

    #[allow(clippy::empty_loop)]
    loop {}
}

mod runtime;

fn test_sysexit() {
    let page_table = vm::user_pagetable();

    // Allocate pages for a user process
    let user_text = {
        let user_text = pagealloc::allocate_virtpage(
            page_table,
            "usertext",
            Entry::rw_user_text(),
            VaMapping::Addr(0x1000),
            RootPageTableType::User,
        )
        .expect("couldn't allocate user_text");

        // Machine code and assembly to call syscall exit
        //   00 00 80 D2    ; mov x0, #0
        //   21 00 80 D2    ; mov x1, #1
        //   61 00 00 D4    ; svc #3
        let proc_text_bytes: [u8; 12] =
            [0x00, 0x00, 0x80, 0xd2, 0x21, 0x00, 0x80, 0xd2, 0x61, 0x00, 0x00, 0xd4];
        user_text.0[..proc_text_bytes.len()].copy_from_slice(&proc_text_bytes);
        user_text
    };
    let user_text_va = user_text as *const _ as u64;

    let user_stack = pagealloc::allocate_virtpage(
        page_table,
        "userstack",
        Entry::rw_user_data(),
        VaMapping::Addr(KZERO - 0x1000),
        RootPageTableType::User,
    )
    .expect("couldn't allocate user_stack");

    // Executing user process!
    println!("Executing user process");
    let proc_stack = unsafe { core::slice::from_raw_parts_mut(user_stack, 4096) };

    // Initialise a Context struct on the process stack, at the end of the proc_stack_buffer.
    let ps_addr = &proc_stack as *const _ as u64;
    let proc_stack_initial_ctx = ps_addr + (4096 - size_of::<swtch::Context>()) as u64;
    let proc_context_ptr: *mut swtch::Context = proc_stack_initial_ctx as *mut swtch::Context;

    // Need to push a context object onto the stack, with x30 populated at the
    // address of proc_textbuf
    let proc_context_ref: &mut swtch::Context = unsafe { &mut *proc_context_ptr };
    proc_context_ref.set_stack_pointer(&proc_context_ptr as *const _ as u64);
    proc_context_ref.set_return(user_text_va);

    let mut kernel_context: *mut swtch::Context = null_mut();
    let kernel_context_ptr: *mut *mut swtch::Context = &mut kernel_context;

    //println!("proc ctx: {:#?}", proc_context_ref);

    unsafe { swtch::swtch(kernel_context_ptr, &*proc_context_ptr) };

    //println!("x30: {:#016x}", proc_context_ref.x30);
}
