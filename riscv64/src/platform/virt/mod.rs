use port::fdt::DeviceTree;

use crate::{
    address::{PhysicalAddress, VirtualAddress},
    kmem::get_boot_page_table,
    paging,
};

pub mod devcons;

pub const PHYSICAL_MEMORY_OFFSET: usize = 0xFFFF_FFFF_4000_0000;

pub const PGSIZE: usize = 4096; // bytes per page
pub const PGSHIFT: usize = 12; // bits of offset within a page
pub const PGMASKLEN: usize = 9;
pub const PGMASK: usize = 0x1FF;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("boot_page_table.S"),);

pub fn platform_init(dt: &DeviceTree) {
    let root = get_boot_page_table();

    if let Some(ns16550a_reg) = dt
        .find_compatible("ns16550a")
        .next()
        .and_then(|uart| dt.property_translated_reg_iter(uart).next())
        .and_then(|reg| reg.regblock())
    {
        port::println!("mapping serial port to 0x{:X}", ns16550a_reg.addr);
        paging::map(
            root,
            VirtualAddress::new(ns16550a_reg.addr as usize),
            PhysicalAddress::new(ns16550a_reg.addr as usize),
            paging::EntryBits::ReadWrite.val(),
            0, // level 0 = 4k page
        );
    }
}
