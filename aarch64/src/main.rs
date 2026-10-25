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
use port::dat::{MMU, Perm, ProcArch, PutError};
use port::fdt::DeviceTree;
use port::mem::{PAGE_SIZE_4K, PhysAddr, PhysRange, VirtRange};
use port::println;
use port::segment;
use proc::Arm64;
use reg::midr_el1::MidrEl1;
use swtch::Label;
use vm::RootPageTableType;

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
    test_user_processes();
}

mod runtime;

// Test programs for test_user_processes.  Each is copied to UTZERO from
// between its start and end labels, so it must be position-independent, and
// starts 8-byte aligned so its literals stay aligned.  The syscall ABI is
// svc #0, with the number in x8, arguments in x0-x5, and the result in x0.
#[cfg(not(test))]
core::arch::global_asm!(
    ".section .rodata.usertest, \"a\"",
    // SYSR1 returns 0, then exits(nil), or exits("sysr1") if it didn't.
    ".balign 8",
    "usertest_sysr1:",
    "   mov x0, #41",
    "   mov x8, #{SYSR1}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 3f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .asciz \"sysr1\"",
    "usertest_sysr1_end:",
    // exits("bye")
    ".balign 8",
    "usertest_bye:",
    "   adr x0, 1f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    "1: .asciz \"bye\"",
    "usertest_bye_end:",
    // An unknown syscall.
    ".balign 8",
    "usertest_badsys:",
    "   mov x8, #9999",
    "   svc #0",
    "usertest_badsys_end:",
    // exits with an unmapped user address, the page after the text.
    ".balign 8",
    "usertest_unmapped:",
    "   ldr x0, 1f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    ".balign 8",
    "1: .quad {UNMAPPED}",
    "usertest_unmapped_end:",
    // exits with a kernel address.
    ".balign 8",
    "usertest_kernel:",
    "   ldr x0, 1f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    ".balign 8",
    "1: .quad {KZERO}",
    "usertest_kernel_end:",
    // pwrite(1, "hello\n", 6, -1) returns 6, then exits(nil), or
    // exits("pwrite") if it didn't.
    ".balign 8",
    "usertest_hello:",
    "   mov x0, #1",
    "   adr x1, 3f",
    "   mov x2, #6",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmp x0, #6",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 4f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .ascii \"hello\\n\"",
    "4: .asciz \"pwrite\"",
    "usertest_hello_end:",
    // pwrite to fd 3 returns -1, then exits(nil), or exits("pwrite fd") if it
    // didn't.
    ".balign 8",
    "usertest_badfd:",
    "   mov x0, #3",
    "   adr x1, 4f",
    "   mov x2, #1",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmn x0, #1",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 3f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .asciz \"pwrite fd\"",
    "4: .ascii \"x\"",
    "usertest_badfd_end:",
    // pwrite from an unmapped user address, the page after the text.
    ".balign 8",
    "usertest_badbuf:",
    "   mov x0, #1",
    "   ldr x1, 1f",
    "   mov x2, #1",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    ".balign 8",
    "1: .quad {UNMAPPED}",
    "usertest_badbuf_end:",
    // brk_(0) returns the bss's start.  brk_ to just past its second page
    // returns 0, and both pages keep what's stored in every word.  Shrinking
    // frees them, and growing again gets pages that read zero in every word,
    // though they were likely the pages just freed.  Then exits(nil), or
    // exits("brk") if any of that didn't happen.
    ".balign 8",
    "usertest_brk:",
    "   ldr x19, 9f",
    "   add x20, x19, #2, lsl #12",
    "   mov x0, #0",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cmp x0, x19",
    "   b.ne 1f",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   mov x1, x19",
    "3: str x19, [x1], #8",
    "   cmp x1, x20",
    "   b.ne 3b",
    "   mov x1, x19",
    "4: ldr x2, [x1], #8",
    "   cmp x2, x19",
    "   b.ne 1f",
    "   cmp x1, x20",
    "   b.ne 4b",
    "   mov x0, x19",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   mov x1, x19",
    "5: ldr x2, [x1], #8",
    "   cbnz x2, 1f",
    "   cmp x1, x20",
    "   b.ne 5b",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 7f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk\"",
    ".balign 8",
    "8: .quad {BSS} + 2 * {PAGE} - 1",
    "9: .quad {BSS}",
    "usertest_brk_end:",
    // Grow the bss by a page and use it, shrink it back, then pwrite from the
    // freed page, which kills the process.  exits("brk shrink") if it
    // doesn't.
    ".balign 8",
    "usertest_brkshrink:",
    "   ldr x19, 9f",
    "   add x0, x19, #1",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   str x19, [x19]",
    "   mov x0, x19",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   mov x0, #1",
    "   mov x1, x19",
    "   mov x2, #1",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "1: adr x0, 7f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk shrink\"",
    ".balign 8",
    "9: .quad {BSS}",
    "usertest_brkshrink_end:",
    // brk_ into the guard page below the stack, and past all memory, return
    // -1, then exits(nil), or exits(...) if they didn't.
    ".balign 8",
    "usertest_brkstack:",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cmn x0, #1",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 7f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk stack\"",
    ".balign 8",
    "8: .quad {GUARD}",
    "usertest_brkstack_end:",
    ".balign 8",
    "usertest_brkmem:",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cmn x0, #1",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 7f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk memory\"",
    ".balign 8",
    "8: .quad {HUGE}",
    "usertest_brkmem_end:",
    SYSR1 = const port::syscall::SYSR1,
    EXITS = const port::syscall::EXITS,
    PWRITE = const port::syscall::PWRITE,
    BRK = const port::syscall::BRK_,
    PAGE = const PAGE_SIZE_4K,
    BSS = const BSS,
    GUARD = const USTKTOP - USTKSIZE - PAGE_SIZE_4K + 1,
    HUGE = const BSS + (64 << 30),
    UNMAPPED = const UTZERO + PAGE_SIZE_4K,
    KZERO = const param::KZERO,
);

/// The text of the test program between labels `$start` and `$end`.
macro_rules! usertext {
    ($start:ident, $end:ident) => {{
        unsafe extern "C" {
            static $start: [u8; 0];
            static $end: [u8; 0];
        }
        // SAFETY: the labels bound one program in .rodata.usertest.
        unsafe {
            let start = $start.as_ptr();
            core::slice::from_raw_parts(start, $end.as_ptr().addr() - start.addr())
        }
    }};
}

/// A test process: its start function, and the status it should exit with.
struct UserTest {
    name: &'static str,
    start: fn(),
    status: &'static [u8],
}

/// Run each test process, checking its exit status, that it freed everything
/// it allocated, and that it reused the first's kernel stack.
fn test_user_processes() {
    let tests = [
        UserTest { name: "sysr1", start: sysr1_start, status: b"" },
        UserTest {
            name: "exits",
            start: || run_user(usertext!(usertest_bye, usertest_bye_end)),
            status: b"bye",
        },
        UserTest {
            name: "bad sys call",
            start: || run_user(usertext!(usertest_badsys, usertest_badsys_end)),
            status: b"sys: bad sys call",
        },
        UserTest {
            name: "unmapped address",
            start: || run_user(usertext!(usertest_unmapped, usertest_unmapped_end)),
            status: b"sys: bad address in syscall",
        },
        UserTest {
            name: "kernel address",
            start: || run_user(usertext!(usertest_kernel, usertest_kernel_end)),
            status: b"sys: bad address in syscall",
        },
        UserTest {
            name: "pwrite",
            start: || run_user(usertext!(usertest_hello, usertest_hello_end)),
            status: b"",
        },
        UserTest {
            name: "pwrite to a bad fd",
            start: || run_user(usertext!(usertest_badfd, usertest_badfd_end)),
            status: b"",
        },
        UserTest {
            name: "pwrite from a bad buffer",
            start: || run_user(usertext!(usertest_badbuf, usertest_badbuf_end)),
            status: b"sys: bad address in syscall",
        },
        UserTest {
            name: "brk",
            start: || run_user(usertext!(usertest_brk, usertest_brk_end)),
            status: b"",
        },
        UserTest {
            name: "brk shrink",
            start: || run_user(usertext!(usertest_brkshrink, usertest_brkshrink_end)),
            status: b"sys: bad address in syscall",
        },
        UserTest {
            name: "brk into the stack",
            start: || run_user(usertext!(usertest_brkstack, usertest_brkstack_end)),
            status: b"",
        },
        UserTest {
            name: "brk past memory",
            start: || run_user(usertext!(usertest_brkmem, usertest_brkmem_end)),
            status: b"",
        },
        UserTest { name: "elf", start: elf_start, status: b"" },
    ];
    let mut first_kstack_top = None;
    for UserTest { name, start, status } in tests {
        let (used_before, _) = port::pagealloc::usage_bytes();
        let p = port::proc::newproc::<Arm64>(start).expect("couldn't create process");
        let kstack_top = p.kstack().top();
        assert_eq!(*first_kstack_top.get_or_insert(kstack_top), kstack_top);
        println!("Executing user process: {name}");
        assert_eq!(port::proc::run(p), status, "{name}");
        assert_eq!(port::pagealloc::usage_bytes().0, used_before, "{name}");
    }
    println!("user process test: ok");
}

/// The first test program's start, which also checks the user access checks
/// against its mappings.
fn sysr1_start() {
    load_user(usertext!(usertest_sysr1, usertest_sysr1_end));
    test_user_access();
    enter_user(UTZERO)
}

fn run_user(text: &[u8]) -> ! {
    load_user(text);
    enter_user(UTZERO)
}

/// The ELF loader's test program (see port/lib/test/elf).
static HELLO_ELF: &[u8] = include_bytes!("../../port/lib/test/elf/hello.elf");

/// Load HELLO_ELF, check its segments' permissions, and enter it.  Its
/// layout is set by hello.ld: a page of text, a page of rodata, then two
/// pages of data, followed by a bss that runs onto a third.
fn elf_start() {
    use port::dat::Access::{Read, Write};
    dirty_free_pages();
    let entry = match port::exec::load::<Arm64>(HELLO_ELF) {
        Ok(entry) => entry,
        Err(err) => port::proc::pexit::<Arm64>(err.as_bytes()),
    };
    let ok = vm::user_page_ok;
    assert!(ok(0x10000, Read) && !ok(0x10000, Write), "elf text");
    assert!(ok(0x11000, Read) && !ok(0x11000, Write), "elf rodata");
    assert!(ok(0x12000, Write) && ok(0x13000, Write), "elf data");
    assert!(ok(0x14000, Write), "elf bss");
    assert!(!ok(0x15000, Read), "above the elf bss");
    map_stack();
    enter_user(entry)
}

/// The test processes' bss starts, empty, at the page after their text.
const BSS: usize = UTZERO + PAGE_SIZE_4K;

/// Map `text` at UTZERO, and a zeroed stack below USTKTOP, in the current
/// process, and give it an empty bss at BSS.
fn load_user(text: &[u8]) {
    assert!(text.len() <= PAGE_SIZE_4K, "user text too big");
    let text_pa = port::pagealloc::allocate_physpage().expect("couldn't allocate user text");
    {
        let k = kmap::kmap(text_pa);
        // SAFETY: kmaps the whole of the newly allocated page, which nothing
        // else refers to.
        let page = unsafe { core::slice::from_raw_parts_mut(k.va() as *mut u8, PAGE_SIZE_4K) };
        page.fill(0);
        page[..text.len()].copy_from_slice(text);
    }
    Arm64::put(UTZERO, text_pa, Perm::Text).expect("couldn't map user text");
    map_stack();
    segment::set_bss::<Arm64>(VirtRange::new(BSS, BSS));
}

/// Fill some free pages with ones, so a loader that doesn't zero what it maps
/// is caught.  The page allocator is next fit, and freeing a page moves its
/// cursor there (BitmapPageAlloc::deallocate), so freeing these last first
/// makes them the next it hands out.  That's checked here, so the test can't
/// silently stop working if the allocator changes.
fn dirty_free_pages() {
    let mut pages = [PhysAddr::new(0); 8];
    for pa in &mut pages {
        *pa = port::pagealloc::allocate_physpage().expect("couldn't allocate a page");
        let k = kmap::kmap(*pa);
        // SAFETY: kmaps the whole of the newly allocated page.
        unsafe { core::ptr::write_bytes(k.va() as *mut u8, 0xff, PAGE_SIZE_4K) };
    }
    for pa in pages.into_iter().rev() {
        port::pagealloc::free_physpage(pa).expect("couldn't free a page");
    }
    let next = port::pagealloc::allocate_physpage().expect("couldn't allocate a page");
    assert_eq!(next, pages[0], "the next page allocated isn't a dirtied one");
    port::pagealloc::free_physpage(next).expect("couldn't free a page");
}

/// Map a zeroed stack below USTKTOP in the current process.
fn map_stack() {
    let stack_range = VirtRange::new(USTKTOP - USTKSIZE, USTKTOP);
    segment::map_zeroed::<Arm64>(stack_range).expect("couldn't allocate user stack");
}

/// Enter the current process at `entry`, with its stack at USTKTOP.
fn enter_user(entry: usize) -> ! {
    // SAFETY: up is this process, which is running, and its context started
    // below the space to_user uses at the top of its stack.
    let kstack_top = unsafe { (*Arm64::up()).kstack().top() };
    // SAFETY: as above, and the caller mapped the text and stack.
    unsafe { trap::to_user(kstack_top, entry as u64, USTKTOP as u64) }
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

/// Map some pages into a new address space, unmap one, then release it,
/// checking that every page comes back.  A map_zeroed that fails partway
/// leaves nothing behind.
fn test_release() {
    use port::dat::Access::Read;
    let (used_before, _) = port::pagealloc::usage_bytes();
    let space = vm::AddressSpace::new().expect("couldn't allocate address space");
    // SAFETY: nothing refers to memory through the empty address space.
    unsafe { vm::switch(&space) };
    let page = |i| UTZERO + i * PAGE_SIZE_4K;

    segment::map_zeroed::<Arm64>(VirtRange::new(page(4), page(5))).expect("couldn't map a page");
    let (used_mid, _) = port::pagealloc::usage_bytes();
    let overlapping = segment::map_zeroed::<Arm64>(VirtRange::new(page(0), page(5)));
    assert_eq!(overlapping, Err(PutError::AlreadyMapped));
    assert_eq!(port::pagealloc::usage_bytes().0, used_mid, "a failed map_zeroed leaked");
    assert!(!vm::user_page_ok(page(0), Read), "a failed map_zeroed left a page mapped");
    assert!(vm::user_page_ok(page(4), Read));

    segment::map_zeroed::<Arm64>(VirtRange::new(page(0), page(4))).expect("couldn't map pages");
    let pa = Arm64::unput(page(1)).expect("unput found nothing");
    assert!(!vm::user_page_ok(page(1), Read), "unput left the page mapped");
    assert_eq!(Arm64::unput(page(1)), None);
    port::pagealloc::free_physpage(pa).expect("couldn't free the unput page");

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
