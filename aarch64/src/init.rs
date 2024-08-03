use crate::devcons;
use crate::kmem;
use crate::kmem::from_virt_to_physaddr;
use crate::kmem::heap_virtrange;
use crate::mailbox;
use crate::pagealloc;
use crate::registers;
use crate::runtime;
use crate::trap;
use crate::vm;
use crate::vm::kernel_root;
use crate::vm::PageTable;
use crate::vmalloc;
use alloc::boxed::Box;
use core::ptr;
use port::fdt::DeviceTree;
use port::mem::{PhysRange, VirtRange};
use port::println;

static mut KPGTBL: PageTable = PageTable::empty();

unsafe fn print_memory_range(name: &str, range: VirtRange) {
    let start = range.start();
    let end = range.end();
    let size = range.size();
    println!("  {name}{start:#x}..{end:#x} ({size:#x})");
}

fn print_binary_sections() {
    println!("Binary sections:");
    unsafe {
        print_memory_range("boottext:\t", kmem::boottext_virtrange());
        print_memory_range("text:\t\t", kmem::text_virtrange());
        print_memory_range("rodata:\t", kmem::rodata_virtrange());
        print_memory_range("data:\t\t", kmem::data_virtrange());
        print_memory_range("bss:\t\t", kmem::bss_virtrange());
        print_memory_range("heap:\t\t", kmem::heap_virtrange());
        print_memory_range("total:\t", kmem::total_virtrange());
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

/// This function is concerned with preparing the system to the point where an
/// allocator can be set up and allocation is available.  We can't assume
/// there's any allocator available when executing this function.
fn init_pre_allocator(dtb_va: usize) {
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
}

pub fn init(dtb_va: usize) {
    init_pre_allocator(dtb_va);

    // From this point we can use the global allocator.  Initially it uses a
    // bump allocator that makes permanent allocations.  This can be used to
    // create the more complex vmem allocator.  Once the vmem allocator is
    // available, we switch to that.
    runtime::enable_bump_allocator();

    vmalloc::init(heap_virtrange());
    //runtime::enable_vmem_allocator();

    let _b = Box::new("ddododo");

    print_memory_info();

    kernel_root().print_recursive_tables();

    println!("looping now");

    {
        let test = vmalloc::alloc(1024);
        println!("test alloc: {:p}", test);
        let test2 = vmalloc::alloc(1024);
        println!("test alloc: {:p}", test2);
    }

    #[allow(clippy::empty_loop)]
    loop {}
}
