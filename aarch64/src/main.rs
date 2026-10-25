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
mod kstack;
mod mach;
mod mailbox;
mod pagealloc;
mod param;
mod pre_mmu;
mod proc;
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
use param::{USTKSIZE, USTKTOP, UTZERO};
use port::dat::ProcArch;
use port::fdt::DeviceTree;
use port::mem::{PAGE_SIZE_4K, PhysRange, VirtRange};
use port::println;
use proc::Arm64;
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
    let (used, total) = port::pagealloc::usage_bytes();
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
    mach::init();
    trap::init();

    // Nothing needs the boot identity map in ttbr0_el1 any more.  Replace it
    // with the empty user root, so low addresses, including null, fault.
    // SAFETY: the empty root hasn't been loaded yet, the other CPUs are
    // parked, and nothing refers to memory through the identity map.
    unsafe {
        vm::init_empty_user_root();
        vm::switch(&vm::AddressSpace::empty());
    }

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
    if let Err(err) = port::pagealloc::init_page_allocator(&dt, physranges.into_iter()) {
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

    vmdebug::print_recursive_tables(RootPageTableType::Kernel);

    println!("Set up a user process");

    let _b = Box::new("ddododo");

    test_kmap();
    test_swtch();
    test_release();
    test_user_process();
}

mod runtime;

/// User text for test_user_process:
///   20 05 80 D2    ; mov x0, #41
///   01 00 00 D4    ; svc #0        (returns x0 + 1)
///   61 00 00 D4    ; svc #3        (exit with x0)
const TEST_USER_TEXT: [u32; 3] = [0xd280_0520, 0xd400_0001, 0xd400_0061];

/// Run a process that makes a syscall and exits with its result, twice,
/// checking that each run frees everything it allocated, so the second reuses
/// the first's kernel stack.
fn test_user_process() {
    let mut first_kstack_top = None;
    for i in 0..2 {
        let (used_before, _) = port::pagealloc::usage_bytes();
        let p = port::proc::newproc::<Arm64>(test_user_start).expect("couldn't create process");
        let kstack_top = p.kstack().top();
        assert_eq!(*first_kstack_top.get_or_insert(kstack_top), kstack_top);
        println!("Executing user process {i}");
        assert_eq!(port::proc::run(p), b"42");
        assert_eq!(port::pagealloc::usage_bytes().0, used_before);
    }
    println!("user process test: ok");
}

/// The test process's start: load TEST_USER_TEXT and enter it.
fn test_user_start() {
    let text_pa = port::pagealloc::allocate_physpage().expect("couldn't allocate user text");
    {
        let k = kmap::kmap(text_pa);
        // SAFETY: kmaps the whole of the newly allocated page, which nothing
        // else refers to.
        let page = unsafe { core::slice::from_raw_parts_mut(k.va() as *mut u32, PAGE_SIZE_4K / 4) };
        page.fill(0);
        page[..TEST_USER_TEXT.len()].copy_from_slice(&TEST_USER_TEXT);
        vm::sync_icache(k.va(), PAGE_SIZE_4K);
    }
    vm::map_phys_range(
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
    pagealloc::map_zeroed_user_pages("userstack", stack_range)
        .expect("couldn't allocate user stack");

    test_user_access();

    // SAFETY: up is this process, which is running, and its context started
    // below the space to_user uses at the top of its stack.
    let kstack_top = unsafe { (*Arm64::up()).kstack().top() };
    // SAFETY: as above, and the text and stack were just mapped.
    unsafe { trap::to_user(kstack_top, UTZERO as u64, USTKTOP as u64) }
}

/// Check user_page_ok against the test process's mappings, which are
/// active: text is read-only, the stack is read-write, and everything else,
/// including the kernel and the recursive window, is inaccessible.
fn test_user_access() {
    use port::dat::Access::{Read, Write};
    let ok = vm::user_page_ok;
    assert!(ok(UTZERO, Read) && !ok(UTZERO, Write), "user text");
    assert!(ok(USTKTOP - 1, Read) && ok(USTKTOP - USTKSIZE, Write), "user stack");
    assert!(!ok(USTKTOP - USTKSIZE - 1, Read), "below the user stack");
    assert!(!ok(0, Read), "null");
    assert!(!ok(param::UADDRTOP, Read), "user recursive window");
    assert!(!ok(param::KZERO, Read), "kernel");
    println!("user access test: ok");
}

/// Write a page that's mapped nowhere else through one kmap, and read it back
/// through another.
fn test_kmap() {
    let pa = port::pagealloc::allocate_physpage().expect("couldn't allocate kmap test page");
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

/// Map some pages into a new address space, then release it, checking that
/// every page comes back.
fn test_release() {
    let (used_before, _) = port::pagealloc::usage_bytes();
    let space = vm::AddressSpace::new().expect("couldn't allocate address space");
    // SAFETY: nothing refers to memory through the empty address space.
    unsafe { vm::switch(&space) };
    let range = VirtRange::new(UTZERO, UTZERO + 4 * PAGE_SIZE_4K);
    pagealloc::map_zeroed_user_pages("releasetest", range).expect("couldn't map pages");

    let mut freed = 0;
    // SAFETY: nothing refers to memory through space after this.
    unsafe {
        vm::release(space, |pa| {
            port::pagealloc::free_physpage(pa).expect("couldn't free a page");
            freed += 1;
        })
    }
    .expect("couldn't release address space");
    assert_eq!(freed, 4);
    // SAFETY: the empty address space is refused, and nothing is freed.
    let empty = unsafe { vm::release(vm::AddressSpace::empty(), |_| {}) };
    assert!(matches!(empty, Err(vm::PageTableError::EmptyAddressSpace)));
    assert_eq!(port::pagealloc::usage_bytes().0, used_before);
    println!("release test: ok");
}

static SWTCH_TEST_MAIN: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());
static SWTCH_TEST_OTHER: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());

/// Switch back and forth between main9 and another kernel context with its
/// own stack, checking that state held across the switches survives.
fn test_swtch() {
    let (used_before, _) = port::pagealloc::usage_bytes();
    let stack = kstack::alloc().expect("couldn't allocate swtch test stack");
    let stack_top = stack.top();

    unsafe { *SWTCH_TEST_OTHER.get() = Label::new_context(swtch_test_other, stack_top) };

    let canary = black_box(0x0123_4567_89ab_cdef_u64);
    for i in 0..3 {
        println!("swtch test: main {i}");
        unsafe { swtch::swtch(SWTCH_TEST_MAIN.get(), SWTCH_TEST_OTHER.get()) };
    }
    assert_eq!(black_box(canary), 0x0123_4567_89ab_cdef);

    // Nothing switches back to the other context, so its stack can be freed.
    // A new stack then reuses the slot, and the pages are all returned.
    // SAFETY: the other context is never resumed.
    unsafe { kstack::free(stack) };
    // Its label now points into a freed stack.  Reset it, so a mistaken
    // switch to it jumps to address 0 and faults.
    unsafe { *SWTCH_TEST_OTHER.get() = Label::new() };
    let again = kstack::alloc().expect("couldn't allocate a second stack");
    assert_eq!(again.top(), stack_top);
    // SAFETY: nothing ran on this stack.
    unsafe { kstack::free(again) };
    assert_eq!(port::pagealloc::usage_bytes().0, used_before);
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
