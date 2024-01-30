#![allow(non_upper_case_globals)]

use crate::{
    kalloc,
    kmem::{ebss_addr, erodata_addr, etext_addr, text_addr, PhysAddr, PhysRange},
    registers::rpi_mmio,
};
use bitstruct::bitstruct;
use core::fmt;
use core::ptr::write_volatile;
use num_enum::{FromPrimitive, IntoPrimitive};
use port::fdt::DeviceTree;

#[cfg(not(test))]
use port::println;

pub const PAGE_SIZE_4K: usize = 4 << 10;
pub const PAGE_SIZE_2M: usize = 2 << 20;
pub const PAGE_SIZE_1G: usize = 1 << 30;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSize {
    Page4K,
    Page2M,
    Page1G,
}

impl PageSize {
    const fn size(&self) -> usize {
        match self {
            PageSize::Page4K => PAGE_SIZE_4K,
            PageSize::Page2M => PAGE_SIZE_2M,
            PageSize::Page1G => PAGE_SIZE_1G,
        }
    }
}

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
pub struct Page4K([u8; PAGE_SIZE_4K]);

impl Page4K {
    pub fn clear(&mut self) {
        unsafe {
            core::intrinsics::volatile_set_memory(&mut self.0, 0u8, 1);
        }
    }

    pub fn scribble(&mut self) {
        unsafe {
            core::intrinsics::volatile_set_memory(self, 0b1010_1010u8, 1);
        }
    }
}

#[derive(Debug, IntoPrimitive, FromPrimitive)]
#[repr(u8)]
pub enum Mair {
    #[num_enum(default)]
    Normal = 0,
    Device = 1,
}

#[derive(Debug, IntoPrimitive, FromPrimitive)]
#[repr(u8)]
pub enum AccessPermission {
    #[num_enum(default)]
    PrivRw = 0,
    AllRw = 1,
    PrivRo = 2,
    AllRo = 3,
}

#[derive(Debug, IntoPrimitive, FromPrimitive)]
#[repr(u8)]
pub enum Shareable {
    #[num_enum(default)]
    Non = 0, // Non-shareable (single core)
    Unpredictable = 1, // Unpredicatable!
    Outer = 2,         // Outer shareable (shared across CPUs, GPU)
    Inner = 3,         // Inner shareable (shared across CPUs)
}

bitstruct! {
    /// AArch64 supports various granule and page sizes.  We assume 48-bit
    /// addresses.  This is documented in the 'Translation table descriptor
    /// formats' section of the Arm Architecture Reference Manual.
    /// The virtual address translation breakdown is documented in the 'Translation
    /// Process' secrtion of the Arm Architecture Reference Manual.
    #[derive(Copy, Clone, PartialEq)]
    #[repr(transparent)]
    pub struct Entry(u64) {
        valid: bool = 0;
        page_or_table: bool = 1;
        mair_index: Mair = 2..5;
        non_secure: bool = 5;
        access_permission: AccessPermission = 6..8;
        shareable: Shareable = 8..10;
        accessed: bool = 10; // Was accessed by code
        addr: u64 = 12..48;
        pxn: bool = 53; // Privileged eXecute Never
        uxn: bool = 54; // Unprivileged eXecute Never
    }
}

impl Entry {
    pub const fn empty() -> Entry {
        Entry(0)
    }

    fn rw_kernel_data() -> Self {
        Entry(0)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    fn ro_kernel_data() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRo)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    fn ro_kernel_text() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRw)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(false)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    fn ro_kernel_device() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRw)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Device)
            .with_valid(true)
    }

    const fn with_phys_addr(self, pa: PhysAddr) -> Self {
        Entry(self.0).with_addr(pa.addr() >> 12)
    }

    /// Return the physical page address pointed to by this entry
    fn phys_page_addr(self) -> PhysAddr {
        PhysAddr::new(self.addr() << 12)
    }

    fn virt_page_addr(self) -> usize {
        self.phys_page_addr().as_virt()
    }

    fn table(self, level: Level) -> bool {
        self.page_or_table() && level != Level::Level3
    }
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entry: {:#x} ", self.addr() << 12)?;
        if self.valid() {
            write!(f, " Valid")?;
        } else {
            write!(f, " Invalid")?;
        }
        if self.page_or_table() {
            write!(f, " Page/Table")?;
        } else {
            write!(f, " Block")?;
        }
        write!(f, " {:?}", self.mair_index())?;
        if self.non_secure() {
            write!(f, " NonSecure")?;
        } else {
            write!(f, " Secure")?;
        }
        write!(f, " {:?} {:?}", self.access_permission(), self.shareable())?;
        if self.accessed() {
            write!(f, " Accessed")?;
        }
        if self.pxn() {
            write!(f, " PXN")?;
        }
        if self.uxn() {
            write!(f, " UXN")?;
        }
        Ok(())
    }
}

/// Levels start at the lowest number (most significant) and increase from
/// there.  Four levels would support (for example) 4kiB granules with 4KiB
/// pages using Level0 - Level3, while three would support 2MiB pages with the
/// same size granules, using only Level0 - Level2.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Level0,
    Level1,
    Level2,
    Level3,
}

impl Level {
    /// Returns the next level to translate
    pub fn next(&self) -> Option<Level> {
        match self {
            Level::Level0 => Some(Level::Level1),
            Level::Level1 => Some(Level::Level2),
            Level::Level2 => Some(Level::Level3),
            Level::Level3 => None,
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            Level::Level0 => 0,
            Level::Level1 => 1,
            Level::Level2 => 2,
            Level::Level3 => 3,
        }
    }
}

pub fn va_index(va: usize, level: Level) -> usize {
    match level {
        Level::Level0 => (va >> 39) & 0x1ff,
        Level::Level1 => (va >> 30) & 0x1ff,
        Level::Level2 => (va >> 21) & 0x1ff,
        Level::Level3 => (va >> 12) & 0x1ff,
    }
}

/// Returns a tuple of page table indices for the given virtual address
#[cfg(test)]
fn va_indices(va: usize) -> (usize, usize, usize, usize) {
    (
        va_index(va, Level::Level0),
        va_index(va, Level::Level1),
        va_index(va, Level::Level2),
        va_index(va, Level::Level3),
    )
}

/// Return the virtual address for the page table at level `level` for the
/// given virtual address, assuming the use of recursive page tables.
fn recursive_table_addr(va: usize, level: Level) -> usize {
    let indices_mask = 0x0000_ffff_ffff_f000;
    let indices = va & indices_mask;
    let shift = match level {
        Level::Level0 => 36,
        Level::Level1 => 27,
        Level::Level2 => 18,
        Level::Level3 => 9,
    };
    let recursive_indices = match level {
        Level::Level0 => (511 << 39) | (511 << 30) | (511 << 21) | (511 << 12),
        Level::Level1 => (511 << 39) | (511 << 30) | (511 << 21),
        Level::Level2 => (511 << 39) | (511 << 30),
        Level::Level3 => 511 << 39,
    };
    0xffff_0000_0000_0000 | recursive_indices | ((indices >> shift) & indices_mask)
}

#[derive(Debug)]
pub enum PageTableError {
    AllocationFailed(kalloc::Error),
    EntryIsNotTable,
    PhysRangeIsZero,
}

impl From<kalloc::Error> for PageTableError {
    fn from(err: kalloc::Error) -> PageTableError {
        PageTableError::AllocationFailed(err)
    }
}

#[repr(C, align(4096))]
pub struct Table {
    entries: [Entry; 512],
}

impl Table {
    /// Return a mutable entry from the table based on the virtual address and
    /// the level.  (It uses the level to extract the index from the correct
    /// part of the virtual address).
    pub fn entry_mut(&mut self, level: Level, va: usize) -> Result<&mut Entry, PageTableError> {
        let idx = va_index(va, level);
        Ok(&mut self.entries[idx])
    }

    /// Return the next table in the walk.  If it doesn't exist, create it.
    fn next_mut(&mut self, level: Level, va: usize) -> Result<&mut Table, PageTableError> {
        // Try to get a valid page table entry.  If it doesn't exist, create it.
        let index = va_index(va, level);
        let mut entry = self.entries[index];
        if !entry.valid() {
            // Create a new page table and write the entry into the parent table
            let table = Self::alloc_pagetable()?;
            entry = Entry::rw_kernel_data()
                .with_phys_addr(PhysAddr::from_ptr(table))
                .with_page_or_table(true);
            unsafe {
                write_volatile(&mut self.entries[index], entry);
            }
        }

        if !entry.table(level) {
            return Err(PageTableError::EntryIsNotTable);
        }

        // Return the address of the next table as a recursive address
        let recursive_page_addr = recursive_table_addr(va, level.next().unwrap());
        Ok(unsafe { &mut *(recursive_page_addr as *mut Table) })
    }

    fn alloc_pagetable() -> Result<&'static mut Table, PageTableError> {
        let page = kalloc::alloc()?;
        page.clear();
        Ok(unsafe { &mut *(page as *mut Page4K as *mut Table) })
    }
}

pub type PageTable = Table;

impl PageTable {
    pub const fn empty() -> PageTable {
        PageTable { entries: [Entry::empty(); 512] }
    }

    /// Ensure there's a mapping from va to entry, creating any intermediate
    /// page tables that don't already exist.  If a mapping already exists,
    /// replace it.
    fn map_to(
        &mut self,
        entry: Entry,
        va: usize,
        page_size: PageSize,
    ) -> Result<(), PageTableError> {
        // We change the last entry of the root page table to the address of
        // self for the duration of this method.  This allows us to work with
        // this hierarchy of pagetables even if it's not the current translation
        // table.  We *must* return it to its original state on exit.
        // TODO Only do this if self != kernel_root()
        let old_recursive_entry = kernel_root().entries[511];
        let temp_recursive_entry = Entry::rw_kernel_data()
            .with_phys_addr(PhysAddr::from_ptr(self))
            .with_page_or_table(true);

        unsafe {
            write_volatile(&mut kernel_root().entries[511], temp_recursive_entry);
            // TODO Need to invalidate the single cache entry
            invalidate_all_tlb_entries();
        };

        let dest_entry = match page_size {
            PageSize::Page4K => self
                .next_mut(Level::Level0, va)
                .and_then(|t1| t1.next_mut(Level::Level1, va))
                .and_then(|t2| t2.next_mut(Level::Level2, va))
                .and_then(|t3| t3.entry_mut(Level::Level3, va)),
            PageSize::Page2M => self
                .next_mut(Level::Level0, va)
                .and_then(|t1| t1.next_mut(Level::Level1, va))
                .and_then(|t2| t2.entry_mut(Level::Level2, va)),
            PageSize::Page1G => {
                self.next_mut(Level::Level0, va).and_then(|t1| t1.entry_mut(Level::Level1, va))
            }
        };

        // Entries at level 3 should have the page flag set
        let entry =
            if page_size == PageSize::Page4K { entry.with_page_or_table(true) } else { entry };

        unsafe {
            write_volatile(dest_entry?, entry);
            // Return the recursive entry to its original state
            write_volatile(&mut kernel_root().entries[511], old_recursive_entry);
            // TODO Need to invalidate the single cache entry (+ optionally the recursive entry)
            invalidate_all_tlb_entries();
        }

        Ok(())
    }

    /// Map the physical range using the requested page size.
    /// This aligns on page size boundaries, and rounds the requested range so
    /// that both the alignment requirements are met and the requested range are
    /// covered.
    /// TODO Assuming some of these requests are dynamic, but should not fail,
    /// we should fall back to the smaller page sizes if the requested size
    /// fails.
    pub fn map_phys_range(
        &mut self,
        range: &PhysRange,
        entry: Entry,
        page_size: PageSize,
    ) -> Result<(usize, usize), PageTableError> {
        let mut startva = None;
        let mut endva = 0;
        for pa in range.step_by_rounded(page_size.size()) {
            let va = pa.as_virt();
            self.map_to(entry.with_phys_addr(pa), va, page_size)?;
            startva.get_or_insert(va);
            endva = va + page_size.size();
        }
        startva.map(|startva| (startva, endva)).ok_or(PageTableError::PhysRangeIsZero)
    }

    /// Recursively write out all the tables and all its children
    pub fn print_recursive_tables(&self) {
        println!("Root va:{:p}", self);
        self.print_table_at_level(Level::Level0, 0xffff_ffff_ffff_f000);
    }

    /// Recursively write out the table and all its children
    fn print_table_at_level(&self, level: Level, table_va: usize) {
        let indent = 2 + level.depth() * 2;
        println!("{:indent$}Table {:?} va:{:p}", "", level, self);
        for (i, &pte) in self.entries.iter().enumerate() {
            if pte.valid() {
                print_pte(indent, i, level, pte);

                // Recurse into child table (unless it's the recursive index)
                if i != 511 && pte.table(level) {
                    let next_nevel = level.next().unwrap();
                    let child_va = (table_va << 9) | (i << 12);
                    let child_table = unsafe { &*(child_va as *const PageTable) };
                    child_table.print_table_at_level(next_nevel, child_va);
                }
            }
        }
    }
}

impl fmt::Debug for PageTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", (self as *const Self).addr())
    }
}

/// Helper to print out PTE as part of a table
fn print_pte(indent: usize, i: usize, level: Level, pte: Entry) {
    if pte.table(level) {
        println!("{:indent$}[{:03}] Table {:?} (pte:{:#016x})", "", i, pte, pte.0,);
    } else {
        println!(
            "{:indent$}[{:03}] Entry va:{:#018x} -> {:?} (pte:{:#016x})",
            "",
            i,
            pte.virt_page_addr(),
            pte,
            pte.0,
        );
    }
}

pub unsafe fn init(_dt: &DeviceTree, kpage_table: &mut PageTable, dtb_range: PhysRange) {
    // We use recursive page tables, but we have to be careful in the init call,
    // since the kpage_table is not currently pointed to by ttbr1_el1.  Any
    // recursive addressing of (511, 511, 511, 511) always points to the
    // physical address of the root page table, which isn't what we want here
    // because kpage_table hasn't been switched to yet.

    // Write the recursive entry
    unsafe {
        let entry = Entry::rw_kernel_data()
            .with_phys_addr(PhysAddr::from_ptr(kpage_table))
            .with_page_or_table(true);
        write_volatile(&mut kpage_table.entries[511], entry);
    }

    // TODO We don't actualy unmap the first page...  We should to achieve:
    // Note that the first page is left unmapped to try and
    // catch null pointer dereferences in unsafe code: defense
    // in depth!
    let custom_map = {
        let text_range =
            PhysRange(PhysAddr::from_virt(text_addr())..PhysAddr::from_virt(etext_addr()));
        let data_range = PhysRange::with_len(
            PhysAddr::from_virt(etext_addr()).addr(),
            erodata_addr() - etext_addr(),
        );
        let bss_range = PhysRange::with_len(
            PhysAddr::from_virt(erodata_addr()).addr(),
            ebss_addr() - erodata_addr(),
        );

        let mmio_range = rpi_mmio().expect("mmio base detect failed");

        let mut map = [
            ("DTB", dtb_range, Entry::ro_kernel_data(), PageSize::Page4K),
            ("Kernel Text", text_range, Entry::ro_kernel_text(), PageSize::Page2M),
            ("Kernel Data", data_range, Entry::ro_kernel_data(), PageSize::Page2M),
            ("Kernel BSS", bss_range, Entry::rw_kernel_data(), PageSize::Page2M),
            ("MMIO", mmio_range, Entry::ro_kernel_device(), PageSize::Page2M),
        ];
        map.sort_by_key(|a| a.1.start());
        map
    };

    println!("Memory map:");
    for (name, range, flags, page_size) in custom_map.iter() {
        let mapped_range =
            kpage_table.map_phys_range(range, *flags, *page_size).expect("init mapping failed");
        println!(
            "  {:14}{:#018x}-{:#018x} to {:#018x}-{:#018x} flags: {:?} page_size: {:?}",
            name,
            range.start().addr(),
            range.end().addr(),
            mapped_range.0,
            mapped_range.1,
            flags,
            page_size
        );
    }
}

/// Return the root kernel page table physical address
fn ttbr1_el1() -> u64 {
    #[cfg(not(test))]
    {
        let mut addr: u64;
        unsafe {
            core::arch::asm!("mrs {value}, ttbr1_el1", value = out(reg) addr);
        }
        addr
    }
    #[cfg(test)]
    0
}

// TODO this should just call invalidate_all_tlb_entries afterwards?
#[allow(unused_variables)]
pub unsafe fn switch(kpage_table: &PageTable) {
    #[cfg(not(test))]
    unsafe {
        let pt_phys = PhysAddr::from_ptr(kpage_table).addr();
        // https://forum.osdev.org/viewtopic.php?t=36412&p=303237
        core::arch::asm!(
            "msr ttbr1_el1, {pt_phys}",
            "tlbi vmalle1is", // invalidate all TLB entries
            "dsb ish",      // ensure write has completed
            "isb",          // synchronize context and ensure that no instructions
                            // are fetched using the old translation
            pt_phys = in(reg) pt_phys);
    }
}

#[allow(unused_variables)]
pub unsafe fn invalidate_all_tlb_entries() {
    #[cfg(not(test))]
    unsafe {
        // https://forum.osdev.org/viewtopic.php?t=36412&p=303237
        core::arch::asm!(
            "tlbi vmalle1is", // invalidate all TLB entries
            "dsb ish",        // ensure write has completed
            "isb"
        ); // synchronize context and ensure that no instructions
           // are fetched using the old translation
    }
}

/// Return the root kernel page table
pub fn kernel_root() -> &'static mut PageTable {
    unsafe { &mut *PhysAddr::new(ttbr1_el1()).as_ptr_mut::<PageTable>() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_break_down_va() {
        assert_eq!(va_indices(0xffff8000049fd000), (256, 0, 36, 509));
    }

    #[test]
    fn test_to_use_for_debugging_vaddrs() {
        assert_eq!(va_indices(0xffff8000049fd000), (256, 0, 36, 509));
    }

    #[test]
    fn test_recursive_table_addr() {
        assert_eq!(va_indices(0xffff800008000000), (256, 0, 64, 0));
        assert_eq!(
            va_indices(recursive_table_addr(0xffff800008000000, Level::Level0)),
            (511, 511, 511, 511)
        );
        assert_eq!(
            va_indices(recursive_table_addr(0xffff800008000000, Level::Level1)),
            (511, 511, 511, 256)
        );
        assert_eq!(
            va_indices(recursive_table_addr(0xffff800008000000, Level::Level2)),
            (511, 511, 256, 0)
        );
        assert_eq!(
            va_indices(recursive_table_addr(0xffff800008000000, Level::Level3)),
            (511, 256, 0, 64)
        );
    }
}
