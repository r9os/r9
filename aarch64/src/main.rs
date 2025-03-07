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

extern crate alloc;

use crate::kmem::from_virt_to_physaddr;
use alloc::boxed::Box;
use core::ptr;
use kmem::{boottext_range, bss_range, data_range, rodata_range, text_range, total_kernel_range};
use param::KZERO;
use port::fdt::DeviceTree;
use port::mem::PhysRange;
use port::println;
use vm::{Entry, RootPageTable, RootPageTableType, VaMapping};

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

static mut KERNEL_PAGETABLE: RootPageTable = RootPageTable::empty();
static mut USER_PAGETABLE: RootPageTable = RootPageTable::empty();

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
#[unsafe(no_mangle)]
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
        vm::init_kernel_page_tables(
            &mut *ptr::addr_of_mut!(KERNEL_PAGETABLE),
            dtb_range,
            mailbox::get_arm_memory(),
        );
        vm::switch(&*ptr::addr_of!(KERNEL_PAGETABLE), RootPageTableType::Kernel);

        vm::init_user_page_tables(&mut *ptr::addr_of_mut!(USER_PAGETABLE));
        vm::switch(&*ptr::addr_of!(USER_PAGETABLE), RootPageTableType::User);
    }

    // From this point we can use the global allocator

    print_memory_info();

    vm::print_recursive_tables(RootPageTableType::Kernel);
    vm::print_recursive_tables(RootPageTableType::User);

    {
        let page_table = unsafe { &mut *ptr::addr_of_mut!(KERNEL_PAGETABLE) };
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
                Ok(_allocated_page) => {
                    //         let pa = allocated_page.pa;
                    //         let va = allocated_page.page.data().as_ptr() as *const _ as u64;
                    //         println!("page pa: {pa:?} va: {va:#016x}");

                    //allocated_page.clear();

                    //         let range = PhysRange::new(pa, pa + PAGE_SIZE_4K as u64);
                    //         let entry = Entry::rw_user_text();
                    //         let page_size = PageSize::Page4K;

                    //         //let kpgtable = unsafe { &mut *ptr::addr_of_mut!(KPGTBL) };
                    //         //kernel_root().map_phys_range(&range, entry, page_size).expect("dynamic mapping failed");
                }
                Err(err) => {
                    println!("Error allocating page in kernel space ({i}): {:?}", err);
                    break;
                }
            }
        }
    }

    vm::print_recursive_tables(RootPageTableType::Kernel);
    vm::print_recursive_tables(RootPageTableType::User);

    println!("Now try user space");

    {
        let page_table = unsafe { &mut *ptr::addr_of_mut!(USER_PAGETABLE) };
        let entry = Entry::rw_user_text();
        for i in 0..100 {
            let alloc_result = pagealloc::allocate_virtpage(
                page_table,
                "testuser",
                entry,
                VaMapping::Addr((i + 1) * 4096),
                RootPageTableType::User,
            );
            match alloc_result {
                Ok(_allocated_page) => {}
                Err(err) => {
                    println!("Error allocating page in user space ({i}): {:?}", err);
                    break;
                }
            }
        }
    }

    vm::print_recursive_tables(RootPageTableType::Kernel);
    vm::print_recursive_tables(RootPageTableType::User);

    // test_sysexit();

    let _b = Box::new("ddododo");

    println!("looping now");

    #[allow(clippy::empty_loop)]
    loop {}
}

// struct Proc {}

// static mut PROC: Proc = Proc {};

// fn test_sysexit() {
//     // TODO
//     // Jump to user mode (EL0)
//     // Return to kernel mode (EL1)
//     // Create and switch process stack

//     // Populate process
//     // - page for program code
//     //   - syscall to exit
//     // - page for stack
//     // We need to jump to user mode (EL0)
//     // svc jumps to supervisor mode (EL1)

//     // point to proc page table
//     // switch to process
//     // point to kernel page table

//     // For this hack, we don't need to change page tables.
//     // Instead, we will:
//     // 1. create a buffer for our process
//     // 2. copy code to sysexit
//     // 3. context switch to the process
//     // Machine code and assembly to call syscall exit
//     //   00 00 80 D2    ; mov x0, #0
//     //   21 00 80 D2    ; mov x1, #1
//     //   01 00 00 D4    ; svc #0
//     let proc_text_bytes: [u8; 12] =
//         [0x00, 0x00, 0x80, 0xd2, 0x21, 0x00, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4];
//     let proc_textbuf = unsafe {
//         core::slice::from_raw_parts_mut(
//             alloc::alloc::alloc_zeroed(Layout::from_size_align_unchecked(4096, 4096)),
//             4096,
//         )
//     };
//     proc_textbuf[..proc_text_bytes.len()].copy_from_slice(&proc_text_bytes);

//     let proc_stack_buffer =
//         unsafe { alloc::alloc::alloc_zeroed(Layout::from_size_align_unchecked(4096, 4096)) };
//     let proc_stack = unsafe { core::slice::from_raw_parts_mut(proc_stack_buffer, 4096) };

//     // Initialise a Context struct on the process stack, at the end of the proc_stack_buffer.
//     let proc_stack_initial_ctx =
//         unsafe { proc_stack_buffer.add(4096 - size_of::<swtch::Context>()) };
//     let proc_context_ptr: *mut swtch::Context =
//         proc_stack_initial_ctx as *const _ as *mut swtch::Context;

//     // TODO Set up proc stack
//     // Need to push a context object onto the stack, with x30 populated at the
//     // address of proc_textbuf
//     let proc_context_ref: &mut swtch::Context = unsafe { &mut *proc_context_ptr };
//     proc_context_ref.set_stack_pointer(&proc_context_ptr as *const _ as u64);
//     proc_context_ref.set_return(&proc_textbuf.as_ptr() as *const _ as u64);

//     // let mut foo: *mut swtch::Context = &mut context1;
//     let mut kernel_context: *mut swtch::Context = null_mut();
//     let kernel_context_ptr: *mut *mut swtch::Context = &mut kernel_context;

//     println!("proc ctx: {:?}", proc_context_ref);

//     //context2.set_return(&proc_textbuf as *const _ as u64);
//     unsafe { swtch::swtch(kernel_context_ptr, &*proc_context_ptr) };

//     //println!("x30: {:#016x}", proc_context_ref.x30);
// }

mod runtime;
