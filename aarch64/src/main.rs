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
mod boottest;
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
#[cfg(all(feature = "semihosting", not(test)))]
mod semihosting;
mod swtch;
mod trap;
mod uartmini;
mod uartpl011;
mod vm;
mod vmdebug;

extern crate alloc;

use core::cell::SyncUnsafeCell;
use port::fdt::DeviceTree;
use port::mem::{PhysAddr, PhysRange, VirtRange};
use port::println;
use proc::{Arm64, enter_user};
use reg::midr_el1::MidrEl1;
use swtch::Label;
use vm::RootPageTableType;

use crate::kmem::{
    boottext_physrange, bss_physrange, data_physrange, rodata_physrange, text_physrange,
    total_kernel_physrange,
};
use crate::vm::PageSize;

#[cfg(not(test))]
core::arch::global_asm!(
    include_str!("l.S"),
    STACKSZ = const param::KSTACKSIZE,
    HCR_EL2 = const aarch64_cpu::registers::HCR_EL2::RW::EL1IsAarch64.value,
    SPSR_EL2 = const {
        use aarch64_cpu::registers::SPSR_EL2::{A, D, F, I, M};
        M::EL1h.value | D::Masked.value | A::Masked.value | I::Masked.value | F::Masked.value
    },
    CPACR_EL1 = const aarch64_cpu::registers::CPACR_EL1::FPEN::TrapNothing.value,
    CPTR_EL2 = const CPTR_EL2_NO_FP_TRAP,
    SCTLR_EL1_MMU_OFF = const SCTLR_EL1_RES1,
    SCTLR_EL1_MMU_ON = const {
        use aarch64_cpu::registers::SCTLR_EL1::{C, I, M, SA, SA0};
        SCTLR_EL1_RES1
            | M::Enable.value
            | C::Cacheable.value
            | I::Cacheable.value
            | SA::Enable.value
            | SA0::Enable.value
    },
);

/// SCTLR_EL1's bits that are RES1 on an Armv8.0 CPU, such as the Pi 4's
/// Cortex-A72: LSMAOE (29), nTLSMD (28), SPAN (23), EIS (22), TSCXT (20) and
/// EOS (11) (Arm ARM DDI 0487 M.d, D24.2.174).  Later CPUs keep the Armv8.0
/// behaviour with them set, so Linux's INIT_SCTLR_EL1_MMU_OFF is the same
/// set.  aarch64_cpu has no fields for them.  Every other bit is left clear:
/// little-endian, no alignment checks, and EL0's cache maintenance, CTR_EL0,
/// DC ZVA, DAIF, WFI and WFE trapped to EL1.
#[cfg(not(test))]
const SCTLR_EL1_RES1: u64 = (1 << 29) | (1 << 28) | (1 << 23) | (1 << 22) | (1 << 20) | (1 << 11);

/// CPTR_EL2 with HCR_EL2.E2H clear (D24.2.37): the RES1 bits 13, 9 and 7:0,
/// TZ (8) and TSM (12) set, which trap SVE and SME to EL2 where they're
/// implemented and are RES1 on the A72, and TFP (10) clear, so FP/SIMD
/// doesn't trap.  It's what Linux's __init_el2_cptr writes.  aarch64_cpu has
/// no fields for them.
#[cfg(not(test))]
const CPTR_EL2_NO_FP_TRAP: u64 = 0x33ff;

// kernel.ld asserts that its own KZERO matches this.
#[cfg(not(test))]
core::arch::global_asm!(
    ".globl param_kzero",
    ".set param_kzero, {KZERO}",
    KZERO = const param::KZERO,
);

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
        static overflowstackbase: [u64; 0];
        static overflowstacktop: [u64; 0];
    }

    let base = unsafe { overflowstackbase.as_ptr().addr() };
    let top = unsafe { overflowstacktop.as_ptr().addr() };
    let range = VirtRange::new(base, top);
    let range_size = range.size();
    println!("Overflow stack:\t {range} ({range_size:#x})");
}

/// dtb_pa is the physical address of the DTB structure, which is mapped at
/// KZERO + dtb_pa.
#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_pa: usize) {
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
    let dtb_va = param::KZERO + dtb_pa;
    let dt = unsafe { DeviceTree::from_usize(dtb_va).unwrap() };
    let dtb_physrange = PhysRange::with_pa_len(PhysAddr::new(dtb_pa as u64), dt.size());

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
    println!("sctlr_el1: {:#x}", {
        use aarch64_cpu::registers::{Readable, SCTLR_EL1};
        SCTLR_EL1.get()
    });

    print_stacks();

    print_binary_sections();

    print_board_info();
    print_memory_info();

    vmdebug::print_recursive_tables(RootPageTableType::Kernel);

    // The rest of boot runs on a kernel stack: unlike the boot stack, it has an
    // unmapped guard below it, so an overflow faults, and trap.S reports it.
    on_kernel_stack(main9_rest);
}

/// The rest of main9, on a kernel stack: the boot tests, then echo.
extern "C" fn main9_rest() -> ! {
    boottest::run();

    shutdown(run_echo())
}

/// Run `f` on a newly allocated kernel stack.  The boot context isn't resumed.
fn on_kernel_stack(f: extern "C" fn() -> !) -> ! {
    static BOOT: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());
    static NEXT: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());
    let stack = kstack::alloc().expect("couldn't allocate main9's kernel stack");
    // SAFETY: NEXT is a new context on a stack nothing else uses, and BOOT is
    // only written to.
    unsafe {
        *NEXT.get() = Label::new_context(f, stack.top());
        swtch::swtch(BOOT.get(), NEXT.get());
    }
    unreachable!("the boot context was resumed")
}

/// Stop this CPU once the system has run, or has panicked.  Built for the smoke
/// test, first make QEMU exit, with status 0 if `ok`.
#[cfg(feature = "semihosting")]
pub fn shutdown(ok: bool) -> ! {
    semihosting::exit(if ok { 0 } else { 1 })
}

#[cfg(not(feature = "semihosting"))]
pub fn shutdown(_ok: bool) -> ! {
    halt()
}

/// Stop this CPU for good, waiting for events rather than spinning.
pub fn halt() -> ! {
    loop {
        aarch64_cpu::asm::wfe();
    }
}

mod runtime;

/// echo, built by xtask (user/echo).
static ECHO: &[u8] = include_bytes!(concat!(env!("R9_USER_BIN"), "/echo"));

/// Run echo with argv ["echo", "hello", "world"], as the system's one
/// program, print how it exited, and return whether it exited successfully.  A
/// failure is reported rather than asserted: this is the system running its
/// program, not a test.
fn run_echo() -> bool {
    let p = match port::proc::newproc::<Arm64>(echo_start) {
        Ok(p) => p,
        Err(err) => {
            println!("echo: couldn't create a process: {err:?}");
            return false;
        }
    };
    let status = port::proc::run(p);
    if status.is_empty() {
        println!("echo: exited");
    } else {
        println!("echo: exited with status {}", alloc::string::String::from_utf8_lossy(&status));
    }
    status.is_empty()
}

/// echo's start: exec it with its arguments, and enter it.
fn echo_start() {
    let regs = match port::exec::exec::<Arm64>(ECHO, &[b"echo", b"hello", b"world"]) {
        Ok(regs) => regs,
        Err(err) => port::proc::pexit::<Arm64>(err.as_bytes()),
    };
    enter_user(regs)
}
