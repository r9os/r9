#![feature(alloc_error_handler)]
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

use bit_field::BitField;
use bitflags::{bitflags, Flags};
use port::{print, println};

use crate::{
    memory::phys_to_virt,
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

bitflags! {
    #[derive(Debug)]
    pub struct PageTableFlags: u8 {
        const D = 1 << 7;
        const A = 1 << 6;
        const G = 1 << 5;
        const U = 1 << 4;
        const X = 1 << 3;
        const W = 1 << 2;
        const R = 1 << 1;
        const V = 1;
    }
}

#[derive(Clone, Copy, Debug)]
struct SizedInteger<const N: usize>(u64);

#[derive(Debug)]
struct NumberTooLarge;

impl<const N: usize> TryFrom<u64> for SizedInteger<N> {
    type Error = NumberTooLarge;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if (value.leading_zeros() as usize) < 64 - N {
            return Err(NumberTooLarge);
        }
        Ok(Self(value))
    }
}

impl<const N: usize> From<SizedInteger<N>> for u64 {
    fn from(value: SizedInteger<N>) -> Self {
        value.0
    }
}

#[derive(Debug)]
struct PageTableEntry {
    ppn2: SizedInteger<26>,
    ppn1: SizedInteger<9>,
    ppn0: SizedInteger<9>,
    flags: PageTableFlags,
}

impl PageTableEntry {
    pub fn serialize(&self) -> u64 {
        let mut out = 0u64;
        out.set_bits(0..=7, self.flags.bits() as _);
        out.set_bits(10..=18, self.ppn0.into());
        out.set_bits(19..=27, self.ppn1.into());
        out.set_bits(28..=53, self.ppn2.into());
        out
    }

    pub unsafe fn write_to(&self, addr: u64) {
        unsafe { write_volatile(addr as *mut u64, self.serialize()) }
    }

    pub fn get_vaddr(&self) -> u64 {
        1
    }
}

impl From<u64> for PageTableEntry {
    fn from(value: u64) -> Self {
        let flags = PageTableFlags::from_bits(value.get_bits(0..=7) as _).unwrap();
        Self {
            ppn2: value.get_bits(28..=53).try_into().unwrap(),
            ppn1: value.get_bits(19..=27).try_into().unwrap(),
            ppn0: value.get_bits(10..=18).try_into().unwrap(),
            flags,
        }
    }
}

struct PageTable {
    addr: u64,
}

impl PageTable {
    const ENTRY_SIZE: u64 = 8;

    pub fn new(addr: u64) -> Self {
        Self { addr }
    }

    pub fn get_entry(&self, at: u16) -> PageTableEntry {
        let addr = self.addr + (at as u64 * Self::ENTRY_SIZE);
        let val = unsafe { read_volatile(addr as *const u64) };
        val.into()
    }

    fn next(&self) -> u64 {
        let c_paddr = self.addr;
        let entry = PageTableEntry {
            ppn2: 0x30.try_into().unwrap(),
            ppn1: 0.try_into().unwrap(),
            ppn0: 0.try_into().unwrap(),
            flags: PageTableFlags::W.union(PageTableFlags::R),
        };
        unsafe {
            entry.write_to(c_paddr + 8 * ALLOC_I);
        }
        entry.get_vaddr()
    }
}

static mut ALLOC_I: u64 = 0x100;

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

    extern "C" {
        static boot_page_table: *const c_void;
    }

    let bpt_addr = unsafe { (&boot_page_table) as *const _ as u64 };
    println!("table addr: {:#x}", bpt_addr);
    let bpt = PageTable::new(bpt_addr);
    println!("  0 {:?}", bpt.get_entry(0));
    println!("  1 {:?}", bpt.get_entry(1));
    println!("  2 {:?}", bpt.get_entry(2));
    println!("500 {:?}", bpt.get_entry(500));
    println!("508 {:?}", bpt.get_entry(508));
    println!("509 {:?}", bpt.get_entry(509));
    println!("510 {:?}", bpt.get_entry(510));
    println!("511 {:?}", bpt.get_entry(511));

    #[cfg(not(test))]
    sbi::shutdown();
    #[cfg(test)]
    loop {}
}

#[cfg(test)]
mod tests {
    use crate::{PageTableEntry, PageTableFlags};

    #[test]
    fn test_pagetableentry() {
        {
            let entry = PageTableEntry {
                ppn2: 0.try_into().unwrap(),
                ppn1: 1.try_into().unwrap(),
                ppn0: 2.try_into().unwrap(),
                flags: PageTableFlags::W.union(PageTableFlags::R),
            };
            assert_eq!(entry.serialize(), 0b1_000000010_00_00000110);
        }

        {
            let entry = PageTableEntry {
                ppn2: 0x03f0_0000.try_into().unwrap(),
                ppn1: 1.try_into().unwrap(),
                ppn0: 2.try_into().unwrap(),
                flags: PageTableFlags::W.union(PageTableFlags::R),
            };
            assert_eq!(
                entry.serialize(),
                0b11_1111_0000_0000_0000_0000_0000__000000001__000000010__00__00000110
            );
        }
    }
}
