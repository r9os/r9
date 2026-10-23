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
mod kmap;
mod kmem;
mod mailbox;
mod pagealloc;
mod param;
mod pre_mmu;
mod reg;
mod registers;
mod swtch;
mod trap;
mod uartmini;
mod uartpl011;
mod vm;
mod vmdebug;

extern crate alloc;

use alloc::boxed::Box;
use core::cell::SyncUnsafeCell;
use core::hint::black_box;
use param::{KZERO, USTKSIZE, USTKTOP, UTZERO};
use port::fdt::DeviceTree;
use port::mem::{PAGE_SIZE_4K, PhysRange, VirtRange};
use port::println;
use reg::midr_el1::MidrEl1;
use swtch::Label;
use vm::{Entry, PhysPageAllocator, RootPageTableType, VaMapping, VmTraitImpl};

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

/// dtb_va is the virtual address of the DTB structure, mapped at KZERO + its
/// physical address.
#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    trap::init();

    // Parse the DTB before we set up memory so we can correctly map it
    let dt = unsafe { DeviceTree::from_usize(dtb_va).unwrap() };
    let dtb_physrange = PhysRange::with_pa_len(kmem::paddr(dtb_va as *const u8), dt.size());

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

    println!("Set up a user process");

    let _b = Box::new("ddododo");

    test_kmap();
    test_swtch();

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    test_user_syscalls();
}

mod runtime;

/// A page of user text for test_user_syscalls:
///   20 05 80 D2    ; mov x0, #41
///   01 00 00 D4    ; svc #0        (returns x0 + 1)
///   61 00 00 D4    ; svc #3        (exit)
/// It fills its own page of the kernel image, so it can be mapped into user
/// space read-only without the kernel ever writing it.
#[repr(C, align(4096))]
struct UserTextPage([u32; PAGE_SIZE_4K / 4]);

static TEST_USER_TEXT: UserTextPage = {
    let mut text = [0; PAGE_SIZE_4K / 4];
    text[0] = 0xd280_0520;
    text[1] = 0xd400_0001;
    text[2] = 0xd400_0061;
    UserTextPage(text)
};

/// Check a syscall returns to EL0, then exit.
fn test_user_syscalls() -> ! {
    let page_table = vm::user_pagetable();

    let text_pa = kmem::paddr(&TEST_USER_TEXT);
    page_table
        .map_phys_range(
            &mut PhysPageAllocator {},
            &mut VmTraitImpl {},
            "usertext",
            &PhysRange::with_pa_len(text_pa, PAGE_SIZE_4K),
            VaMapping::Addr(UTZERO),
            Entry::ro_user_text(),
            PageSize::Page4K,
            RootPageTableType::User,
        )
        .expect("couldn't map user text");

    let stack_range = VirtRange::new(USTKTOP - USTKSIZE, USTKTOP);
    pagealloc::map_zeroed_user_pages(page_table, "userstack", stack_range)
        .expect("couldn't allocate user stack");

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
    unsafe { trap::touser(kstack_top, UTZERO as u64, USTKTOP as u64) }
}

/// Write a page that's mapped nowhere else through one kmap, and read it back
/// through another.
fn test_kmap() {
    let pa = pagealloc::allocate_physpage().expect("couldn't allocate kmap test page");
    let pattern = |i: usize| (i * 7 + 3) as u8;

    {
        let k = kmap::kmap(pa);
        // SAFETY: kmaps the whole of the newly allocated page, which nothing
        // else refers to.
        let page = unsafe { core::slice::from_raw_parts_mut(k.va() as *mut u8, PAGE_SIZE_4K) };
        page.iter_mut().enumerate().for_each(|(i, b)| *b = pattern(i));
    }

    let k = kmap::kmap(pa);
    // SAFETY: as above.
    let page = unsafe { core::slice::from_raw_parts(k.va() as *const u8, PAGE_SIZE_4K) };
    assert!(page.iter().enumerate().all(|(i, &b)| b == pattern(i)), "kmap test: page differs");
    println!("kmap test: ok");
}

static SWTCH_TEST_MAIN: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());
static SWTCH_TEST_OTHER: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());

/// Switch back and forth between main9 and another kernel context with its
/// own stack, checking that state held across the switches survives.
fn test_swtch() {
    let stack = pagealloc::allocate_virtpage(
        vm::kernel_pagetable(),
        "swtchstack",
        Entry::rw_kernel_data(),
        VaMapping::Offset(KZERO),
        RootPageTableType::Kernel,
    )
    .expect("couldn't allocate swtch test stack");
    let stack_top = stack as *mut _ as usize + PAGE_SIZE_4K;

    unsafe { *SWTCH_TEST_OTHER.get() = Label::new_context(swtch_test_other, stack_top) };

    let canary = black_box(0x0123_4567_89ab_cdef_u64);
    for i in 0..3 {
        println!("swtch test: main {i}");
        unsafe { swtch::swtch(SWTCH_TEST_MAIN.get(), SWTCH_TEST_OTHER.get()) };
    }
    assert_eq!(black_box(canary), 0x0123_4567_89ab_cdef);
    println!("swtch test: ok");
}

extern "C" fn swtch_test_other() -> ! {
    let mut i = 0;
    loop {
        println!("swtch test: other {i}");
        i += 1;
        unsafe { swtch::swtch(SWTCH_TEST_OTHER.get(), SWTCH_TEST_MAIN.get()) };
    }
}
