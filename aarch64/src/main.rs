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
mod swtch;
mod trap;
mod uartmini;
mod uartpl011;
mod vm;
mod vmdebug;

extern crate alloc;

use port::fdt::DeviceTree;
use port::mem::{PhysAddr, PhysRange, VirtRange};
use port::println;
use proc::{Arm64, enter_user};
use reg::midr_el1::MidrEl1;
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
    SCTLR_EL1_MMU = const {
        use aarch64_cpu::registers::SCTLR_EL1::{C, I, M};
        M::Enable.value | C::Cacheable.value | I::Cacheable.value
    },
);

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
        static interruptstackbase: [u64; 0];
        static interruptstacktop: [u64; 0];
    }

    let interrupt_stack_base = unsafe { interruptstackbase.as_ptr().addr() };
    let interrupt_stack_max = unsafe { interruptstacktop.as_ptr().addr() };
    let range = VirtRange::new(interrupt_stack_base, interrupt_stack_max);
    let range_size = range.size();
    println!("Interrupt stack:\t {range} ({range_size:#x})");
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

    boottest::run();

    run_echo();
}

mod runtime;

/// echo, built by xtask (user/echo).
static ECHO: &[u8] = include_bytes!(concat!(env!("R9_USER_BIN"), "/echo"));

/// Run echo with argv ["echo", "hello", "world"], as the system's one
/// program, and print how it exited.  A failure is reported rather than
/// asserted: this is the system running its program, not a test.
fn run_echo() {
    let p = match port::proc::newproc::<Arm64>(echo_start) {
        Ok(p) => p,
        Err(err) => {
            println!("echo: couldn't create a process: {err:?}");
            return;
        }
    };
    let status = port::proc::run(p);
    if status.is_empty() {
        println!("echo: exited");
    } else {
        println!("echo: exited with status {}", alloc::string::String::from_utf8_lossy(&status));
    }
}

/// echo's start: exec it with its arguments, and enter it.
fn echo_start() {
    let regs = match port::exec::exec::<Arm64>(ECHO, &[b"echo", b"hello", b"world"]) {
        Ok(regs) => regs,
        Err(err) => port::proc::pexit::<Arm64>(err.as_bytes()),
    };
    enter_user(regs)
}
