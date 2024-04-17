#![feature(alloc_error_handler)]
#![feature(stdsimd)]
#![feature(asm_const)]
#![feature(panic_info_message)]
#![feature(ptr_to_from_bits)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod memory;
mod platform;
mod runtime;
mod sbi;
mod uart16550;

use port::{print, println};

use crate::{
    memory::{phys_to_virt, PageTable, PageTableEntry, VirtualAddress},
    platform::{devcons, platform_init},
};
use core::{ffi::c_void, ptr::read_volatile, ptr::write_volatile, slice};
use port::fdt::DeviceTree;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

const LOGO: &str = "
 ️_______________________________________ ️
| ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️|
| ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️💻💻💻 ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️|
| ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️💻 ️ ️ ️ ️ ️💻 ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️|
| ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️💻 ️ ️ ️999 ️ ️💻💻 ️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️|
| ️ ️ ️ ️ ️ ️ ️ ️ ️💻💻💻 ️ ️ ️9 ️ ️ ️9 ️ ️ ️💻💻 ️ ️ ️ ️ ️ ️ ️ ️ ️|
| ️ ️ ️ ️ ️ ️💻💻 ️ ️ ️ ️💻 ️ ️ ️9999 ️ ️ ️ ️ ️💻 ️ ️ ️ ️ ️ ️ ️ ️ ️|
| ️ ️ ️💻💻 ️ ️RRR ️ ️ ️💻 ️ ️ ️ ️ ️9 ️ ️💻️💻💻💻 ️ ️ ️ ️ ️ ️|
| ️💻💻 ️ ️ ️ ️R ️ ️R ️ ️ ️💻 ️999 ️ ️💻 ️ ️ ️ ️ ️ ️💻💻 ️ ️ ️|
| ️💻💻️ ️ ️ ️ ️RRR ️ ️ ️ ️ ️💻️ ️ ️ ️ ️💻 ️ ️⌨️ ️ ️🖱️ ️ ️ ️💻💻 ️|
| ️ ️💻💻️ ️ ️ ️R ️ ️R ️ ️ ️ ️ ️💻️💻️💻️ ️ ️ ️ ️ ️ ️ ️ ️ ️ ️💻💻 ️|
| ️ ️ ️ ️ ️💻💻💻💻💻💻💻💻💻💻💻💻💻💻💻💻 ️ ️|
|_______________________________________|
";

const WHERE_ARE_WE: bool = true;
const WALK_DT: bool = false;

/// debug helper - dump some memory range
pub fn dump(addr: usize, length: usize) {
    let s = unsafe { slice::from_raw_parts(addr as *const u8, length) };
    println!("dump {length} bytes @{addr:x}");
    for w in s.iter() {
        print!("{:02x}", w);
    }
    println!();
}

/// debug helper - dump some memory range, in chunks
pub fn dump_block(base: usize, size: usize, step_size: usize) {
    println!("dump_block {base:x}:{:x}/{step_size:x}", base + size);
    for b in (base..base + size).step_by(step_size) {
        dump(b, step_size);
    }
}

/// nobody wants to write this out, we know it's unsafe...
fn read32(a: usize) -> u32 {
    unsafe { core::ptr::read_volatile(a as *mut u32) }
}

/// nobody wants to write this out, we know it's unsafe...
fn write32(a: usize, v: u32) {
    unsafe { core::ptr::write_volatile(a as *mut u32, v) }
}

/// print a memory range from given pointers - name, start, end, and size
unsafe fn print_memory_range(name: &str, start: &*const c_void, end: &*const c_void) {
    let start = start as *const _ as u64;
    let end = end as *const _ as u64;
    let size = end - start;
    println!("  {name}{start:#x}-{end:#x} ({size:#x})");
}

/// Print binary sections of the kernel: text, rodata, data, bss, total range.
/// NOTE: This needs to align with the corresponding linker script, where the
/// sections are defined.
fn print_binary_sections() {
    extern "C" {
        static text: *const c_void;
        static etext: *const c_void;
        static rodata: *const c_void;
        static erodata: *const c_void;
        static data: *const c_void;
        static edata: *const c_void;
        static bss: *const c_void;
        static end: *const c_void;
    }

    println!("Binary sections:");
    unsafe {
        print_memory_range("text:\t\t", &text, &etext);
        dump((&text) as *const _ as usize, 0x20);
        print_memory_range("rodata:\t", &rodata, &erodata);
        print_memory_range("data:\t\t", &data, &edata);
        print_memory_range("bss:\t\t", &bss, &end);
        print_memory_range("total:\t", &text, &end);
    }
}

fn consume_dt_block(name: &str, a: u64, l: u64) {
    let v = phys_to_virt(a as usize);
    // let v = a as usize;
    println!("- {name}: {a:016x}:{v:016x} ({l:x})");
    match name {
        "test@100000" => {
            let x = read32(v);
            println!("{name}[0]:{x:x}");
            write32(v, x | 0x1234_5678);
            let x = read32(v);
            println!("{name}[0]:{x:x}");
        }
        "uart@10000000" => {
            println!("{name}: {l:x}");
            dump(v, 0x4);
        }
        "plic@c000000" | "clint@2000000" => {
            let x = read32(v);
            println!("{name}[0]:{x:x}");
        }
        "virtio_mmio@10001000" | "virtio_mmio@10002000" => {
            dump(v, 0x20);
        }
        "flash@20000000" => {
            dump(v, 0x20);
        }
        "pci@30000000" => {
            // NOTE: v+l overflows usize, hitting 0
            // dump_block(v, (l - 0x40) as usize, 0x40);
            dump_block(v, 0x40, 0x10);
        }
        _ => {}
    }
}

fn walk_dt(dt: &DeviceTree) {
    dt.nodes().for_each(|n| {
        let name = dt.node_name(&n);
        if false {
            if let Some(name) = name {
                let p = dt.property(&n, "start");
                println!("{name:?}: {n:#?} {p:?}");
            }
        }
        dt.property_translated_reg_iter(n).next().and_then(|i| {
            let b = i.regblock();
            if let Some(b) = b {
                // println!("{b:#?}");
                let a = b.addr;
                if let Some(name) = name {
                    if let Some(l) = b.len {
                        consume_dt_block(name, a, l);
                    }
                }
            }
            b
        });
    });
}

/// check on memory mapping foo
fn where_are_we() {
    let x = "test";
    let p = x.as_ptr();
    // e.g., 0xffffffffc0400096
    println!("YOU ARE HERE (approx.): {p:#x?}");
}

fn flush_tlb() {
    // unsafe { core::arch::riscv64::sinval_vma_all() }
    unsafe { core::arch::asm!("sfence.vma") }
}

#[no_mangle]
pub extern "C" fn main9(hartid: usize, dtb_ptr: u64) -> ! {
    // devcons::init_sbi();
    // println!("\n--> SBI devcons\n");
    // QEMU: dtb@bf000000
    // println!("dtb@{dtb_ptr:x}");
    let dt = unsafe { DeviceTree::from_u64(dtb_ptr).unwrap() };

    devcons::init(&dt);
    println!("\n--> DT / native devcons\n");

    platform_init();
    println!("r9 from the Internet");
    println!("{LOGO}");
    println!("Domain0 Boot HART = {hartid}");
    println!("DTB found at: {dtb_ptr:#x}");
    print_binary_sections();

    if WALK_DT {
        walk_dt(&dt);
    }

    if WHERE_ARE_WE {
        where_are_we();
    }

    println!();
    println!();

    extern "C" {
        static boot_page_table: *const c_void;
    }

    let bpt_addr = unsafe { (&boot_page_table) as *const _ as u64 };
    let bpt = PageTable::new(bpt_addr);
    println!(" boot page table @ 0x{:016x} (0x{:08x})", bpt.get_vaddr(), bpt.get_paddr());

    println!();
    bpt.print_entry(0);
    bpt.print_entry(1);
    bpt.print_entry(2);
    bpt.print_entry(3);
    println!();

    bpt.print_entry(255);
    println!();

    bpt.print_entry(508);
    bpt.print_entry(509);
    bpt.print_entry(510);
    bpt.print_entry(511);
    println!();
    println!();

    // fixed 25 bits, used 39 bits
    // construct 0xffff_ffff_ff00_0000
    // This is where the DTB should be remapped
    const VFIXED: usize = 0xff_ff_ff_80__00_00_00_00;
    let ppn2 = 0x1ff << (9 + 9 + 12);
    let ppn1 = 0x1f8 << (9 + 12);
    let ppn0 = 0; // 0x1ff << 12;
    let poff = 0x0;
    let vaddr = VFIXED | ppn2 | ppn1 | ppn0 | poff;

    // DTB original
    let val0 = read32(dtb_ptr as usize);
    let val0 = u32::from_be(val0);
    println!(" 0x{dtb_ptr:016x}: 0x{val0:08x}");
    // DTB remapped
    let val1 = read32(vaddr);
    let val1 = u32::from_be(val1);
    println!(" 0x{vaddr:016x}: 0x{val1:08x}");

    // Let's create a new PT :)
    println!("=== create new PT");
    let pt_at = 100;
    bpt.print_entry(pt_at);
    let pt = bpt.create_pt_at(pt_at);
    println!(" new pt @ {:016x} ({:08x})", pt.get_vaddr(), pt.get_paddr());
    bpt.print_entry(pt_at);
    println!();

    // Let's create a PTE for the kernel :)
    let kernel_entry_pos = 4;
    bpt.print_entry(kernel_entry_pos);
    // create an entry resolving to the kernel's base addr
    let _ = bpt.create_entry_for(0x8020_0000, kernel_entry_pos);
    bpt.print_entry(kernel_entry_pos);
    println!();

    println!("=== create self reference");
    let self_ref_pos = 5;
    println!(" boot page table before: ");
    bpt.print_entry(self_ref_pos);
    let spt = bpt.create_self_ref(self_ref_pos);
    flush_tlb();
    println!();
    println!(" boot page table after: ");
    bpt.print_entry(self_ref_pos);
    println!(" self reference pt: ");
    spt.print_entry(self_ref_pos);
    println!();
    println!();

    // point to first byte of the kernel
    let vaddr = VirtualAddress {
        vpn2: memory::SizedInteger::<9>(self_ref_pos as u64),
        vpn1: memory::SizedInteger::<9>(kernel_entry_pos as u64),
        vpn0: memory::SizedInteger::<9>(0),
        offset: memory::SizedInteger::<12>(0),
    };
    let va = vaddr.get() as usize;
    println!(" 0x{va:016x} = {vaddr:?}");
    let val = read32(va);
    println!("   0x{val:08x}");
    // write32(va, 0x1234_5678);
    let val = read32(va);
    println!("   0x{val:08x}");
    println!();

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}
