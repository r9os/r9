#![allow(non_upper_case_globals)]

/// Recursive page table implementation for aarch64.
/// Note that currently there are a lot of assumptions that we're dealing with
/// 4KiB tables here, although it supports various sizes of pages.
use crate::{
    kmem::{
        boottext_range, bss_range, data_range, from_ptr_to_physaddr, physaddr_as_ptr_mut,
        rodata_range, text_range,
    },
    pagealloc,
    param::KZERO,
    registers::rpi_mmio,
};
use bitstruct::bitstruct;
use core::fmt;
use core::ptr::write_volatile;
use num_enum::{FromPrimitive, IntoPrimitive};
use port::{
    mem::{PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, PhysAddr, PhysRange},
    pagealloc::PageAllocError,
};

#[cfg(not(test))]
use port::println;

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
pub struct PhysPage4K([u8; PAGE_SIZE_4K]);

impl PhysPage4K {
    pub fn clear(&mut self) {
        unsafe {
            core::intrinsics::volatile_set_memory(&mut self.0, 0u8, 1);
        }
    }
}

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
pub struct VirtPage4K([u8; PAGE_SIZE_4K]);

impl VirtPage4K {}

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
    Unpredictable = 1, // Unpredictable!
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

    pub fn rw_kernel_data() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRw)
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
            .with_access_permission(AccessPermission::PrivRo)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(false)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    fn rw_device() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRw)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Device)
            .with_valid(true)
    }

    pub fn rw_user_text() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::AllRw)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(false)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    const fn with_phys_addr(self, pa: PhysAddr) -> Self {
        Entry(self.0).with_addr(pa.addr() >> 12)
    }

    fn is_table(self, level: Level) -> bool {
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct PteIndices {
    pgtype: RootPageTableType,
    l0: Option<usize>,
    l1: Option<usize>,
    l2: Option<usize>,
    l3: Option<usize>,
}

impl PteIndices {
    #[cfg(test)]
    fn new(
        pgtype: RootPageTableType,
        l0: Option<usize>,
        l1: Option<usize>,
        l2: Option<usize>,
        l3: Option<usize>,
    ) -> Self {
        Self { pgtype, l0, l1, l2, l3 }
    }

    fn none(pgtype: RootPageTableType) -> Self {
        Self { pgtype, l0: None, l1: None, l2: None, l3: None }
    }

    fn with_next_index(&self, i: usize) -> Option<Self> {
        if self.l0.is_none() {
            Some(Self { pgtype: self.pgtype, l0: Some(i), l1: None, l2: None, l3: None })
        } else if self.l1.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: Some(i), l2: None, l3: None })
        } else if self.l2.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: Some(i), l3: None })
        } else if self.l3.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: self.l2, l3: Some(i) })
        } else {
            None
        }
    }

    fn with_last_index(&self, i: usize) -> Option<Self> {
        if self.l0.is_none() {
            None
        } else if self.l1.is_none() {
            Some(Self { pgtype: self.pgtype, l0: Some(i), l1: None, l2: None, l3: None })
        } else if self.l2.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: Some(i), l2: None, l3: None })
        } else if self.l3.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: Some(i), l3: None })
        } else {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: self.l2, l3: Some(i) })
        }
    }

    fn last_index(&self) -> Option<usize> {
        if let Some(i) = self.l3 {
            Some(i)
        } else if let Some(i) = self.l2 {
            Some(i)
        } else if let Some(i) = self.l1 {
            Some(i)
        } else if let Some(i) = self.l0 {
            Some(i)
        } else {
            None
        }
    }

    fn to_va(&self) -> usize {
        let mut va = match self.pgtype {
            RootPageTableType::Kernel => 0xffff_0000_0000_0000,
            RootPageTableType::User => 0x0000_0000_0000_0000,
        };

        va |= if let Some(i) = self.l0 { i << 39 } else { 0 };
        va |= if let Some(i) = self.l1 { i << 30 } else { 0 };
        va |= if let Some(i) = self.l2 { i << 21 } else { 0 };
        va |= if let Some(i) = self.l3 { i << 12 } else { 0 };

        va
    }
}

/// Return the virtual address for the page table at level `level` for the
/// given virtual address, assuming the use of recursive page tables.
fn recursive_table_addr(pgtype: RootPageTableType, va: usize, level: Level) -> usize {
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
    let msbits = match pgtype {
        RootPageTableType::Kernel => 0xffff_0000_0000_0000,
        RootPageTableType::User => 0x0000_0000_0000_0000,
    };
    msbits | recursive_indices | ((indices >> shift) & indices_mask)
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum PageTableError {
    AllocationFailed(PageAllocError),
    EntryIsNotTable,
    PhysRangeIsZero,
    PhysRangeIsNotOnPageBoundary,
}

impl From<PageAllocError> for PageTableError {
    fn from(err: PageAllocError) -> PageTableError {
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
    fn next_mut(
        &mut self,
        pgtype: RootPageTableType,
        level: Level,
        va: usize,
    ) -> Result<&mut Table, PageTableError> {
        // Try to get a valid page table entry.  If it doesn't exist, create it.
        let index = va_index(va, level);
        let mut entry = self.entries[index];
        if !entry.valid() {
            // Create a new page table and write the entry into the parent table
            let page_pa = pagealloc::allocate_physpage();
            //let table = Self::alloc_pagetable();
            let page_pa = match page_pa {
                Ok(p) => p,
                Err(err) => {
                    println!("error:vm:next_mut:can't allocate physpage");
                    return Err(PageTableError::AllocationFailed(err));
                }
            };
            entry = Entry::rw_kernel_data().with_phys_addr(page_pa).with_page_or_table(true);
            unsafe {
                write_volatile(&mut self.entries[index], entry);
            }

            // Clear out the new page
            let recursive_page_addr = recursive_table_addr(pgtype, va, level.next().unwrap());
            let page = unsafe { &mut *(recursive_page_addr as *mut PhysPage4K) };
            page.clear();
        } else {
            if !entry.is_table(level) {
                println!("error:vm:next_mut:entry is not a valid table entry:{entry:?} {level:?}");
                return Err(PageTableError::EntryIsNotTable);
            }
        }

        // Return the address of the next table as a recursive address
        let recursive_page_addr = recursive_table_addr(pgtype, va, level.next().unwrap());
        Ok(unsafe { &mut *(recursive_page_addr as *mut Table) })
    }
}

impl fmt::Debug for Table {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", (self as *const Self).addr())
    }
}

pub enum VaMapping {
    Addr(usize),   // Map to exact virtual address
    Offset(usize), // Map to offset of physical address
}

impl VaMapping {
    fn map(&self, pa: PhysAddr) -> usize {
        match self {
            Self::Addr(va) => *va,
            Self::Offset(offset) => (pa.addr() as usize).wrapping_add(*offset),
        }
    }
}

pub type RootPageTable = Table;

impl RootPageTable {
    pub const fn empty() -> RootPageTable {
        RootPageTable { entries: [Entry::empty(); 512] }
    }

    /// Ensure there's a mapping from va to entry, creating any intermediate
    /// page tables that don't already exist.  If a mapping already exists,
    /// replace it.
    fn map_to(
        &mut self,
        entry: Entry,
        va: usize,
        page_size: PageSize,
        pgtype: RootPageTableType,
    ) -> Result<(), PageTableError> {
        // We change the last entry of the root page table to the address of
        // self for the duration of this method.  This allows us to work with
        // this hierarchy of pagetables even if it's not the current translation
        // table.  We *must* return it to its original state on exit.
        // TODO Only do this if self != kernel_root()
        let old_recursive_entry = root_page_table(pgtype).entries[511];
        let temp_recursive_entry = Entry::rw_kernel_data()
            .with_phys_addr(from_ptr_to_physaddr(self))
            .with_page_or_table(true);

        unsafe {
            write_volatile(&mut root_page_table(pgtype).entries[511], temp_recursive_entry);
            // TODO Need to invalidate the single cache entry
            invalidate_all_tlb_entries();
        };

        let dest_entry = match page_size {
            PageSize::Page4K => self
                .next_mut(pgtype, Level::Level0, va)
                .and_then(|t1| t1.next_mut(pgtype, Level::Level1, va))
                .and_then(|t2| t2.next_mut(pgtype, Level::Level2, va))
                .and_then(|t3| t3.entry_mut(Level::Level3, va)),
            PageSize::Page2M => self
                .next_mut(pgtype, Level::Level0, va)
                .and_then(|t1| t1.next_mut(pgtype, Level::Level1, va))
                .and_then(|t2| t2.entry_mut(Level::Level2, va)),
            PageSize::Page1G => self
                .next_mut(pgtype, Level::Level0, va)
                .and_then(|t1| t1.entry_mut(Level::Level1, va)),
        };
        let dest_entry = match dest_entry {
            Ok(e) => e,
            Err(err) => {
                println!(
                    "error:vm:map_to:couldn't find page table entry. va:{:#x} err:{:?}",
                    va, err
                );
                return Err(err);
            }
        };

        // Entries at level 3 should have the page flag set
        let entry =
            if page_size == PageSize::Page4K { entry.with_page_or_table(true) } else { entry };

        unsafe {
            write_volatile(dest_entry, entry);
            // Return the recursive entry to its original state
            write_volatile(&mut root_page_table(pgtype).entries[511], old_recursive_entry);
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
    /// we should fall back to the smaller page sizes if the requested size fails.
    pub fn map_phys_range(
        &mut self,
        debug_name: &str,
        range: &PhysRange,
        va_mapping: VaMapping,
        entry: Entry,
        page_size: PageSize,
        pgtype: RootPageTableType,
    ) -> Result<(usize, usize), PageTableError> {
        if !range.start().is_multiple_of(page_size.size() as u64)
            || !range.end().is_multiple_of(page_size.size() as u64)
        {
            println!(
                "error:vm:map_phys_range:range not on page boundary. debug_name:{debug_name} range:{range} page_size:{page_size:?}",
            );
            return Err(PageTableError::PhysRangeIsNotOnPageBoundary);
        }

        let mut startva = None;
        let mut endva = 0;
        let mut currva = 0;
        for pa in range.step_by_rounded(page_size.size()) {
            if startva.is_none() {
                currva = va_mapping.map(pa);
                startva = Some(currva);
            } else {
                currva += page_size.size();
            }
            endva = currva + page_size.size();
            self.map_to(entry.with_phys_addr(pa), currva, page_size, pgtype)?;
        }
        startva.map(|startva| (startva, endva)).ok_or(PageTableError::PhysRangeIsZero)
    }
}

/// Return the root user or kernel level page table
pub fn root_page_table(pgtype: RootPageTableType) -> &'static mut RootPageTable {
    let page_table_pa = match pgtype {
        RootPageTableType::User => ttbr0_el1(),
        RootPageTableType::Kernel => ttbr1_el1(),
    };
    unsafe { &mut *physaddr_as_ptr_mut::<RootPageTable>(page_table_pa) }
}

/// Return recursive virtual addresses for the current kernel or user page tables.
/// This depends on the recursive entry of root page tables to have been set up correctly.
pub fn curr_page_table_va(pgtype: RootPageTableType) -> usize {
    match pgtype {
        RootPageTableType::User => 0x0000_ffff_ffff_f000,
        RootPageTableType::Kernel => 0xffff_ffff_ffff_f000,
    }
}

/// Return the current kernel or user page table.
/// This depends on the recursive entry of root page tables to have been set up correctly.
pub fn curr_page_table(pgtype: RootPageTableType) -> &'static mut RootPageTable {
    let ptr = curr_page_table_va(pgtype) as *mut RootPageTable;
    unsafe { &mut *ptr }
}

/// Recursively write out all the tables and all its children
pub fn print_recursive_tables(pgtype: RootPageTableType) {
    let root_page_table = curr_page_table(pgtype);
    println!("Root va:{:018p}", root_page_table);
    print_table_at_level(
        root_page_table,
        Level::Level0,
        curr_page_table_va(pgtype),
        pgtype,
        PteIndices::none(pgtype),
    );
}

/// Recursively write out the table and all its children
fn print_table_at_level(
    page_table: &Table,
    level: Level,
    table_va: usize,
    pgtype: RootPageTableType,
    pte_indices: PteIndices,
) {
    let indent = 2 + level.depth() * 2;
    println!("{:indent$}Table {:?} va:{:018p}", "", level, page_table);

    for i in 0..512 {
        let pte = page_table.entries[i];
        if !pte.valid() {
            continue;
        }

        if !pte.is_table(level) {
            if let Some(pte_indices) = pte_indices.with_last_index(i) {
                print_pte_page(indent, pte_indices, pte);
            }
        } else if i != 511 {
            // Recurse into child table (unless it's the recursive index)
            let child_table_va = match pgtype {
                RootPageTableType::User => ((table_va << 9) | (i << 12)) & 0x0000_ffff_ffff_ffff,
                RootPageTableType::Kernel => (table_va << 9) | (i << 12),
            };
            print_pte_table(indent, i, pte, child_table_va);

            if let Some(next_level_pte_indices) = pte_indices.with_next_index(i) {
                let next_nevel = level.next().unwrap();
                let child_table = unsafe { &*(child_table_va as *const RootPageTable) };
                print_table_at_level(
                    child_table,
                    next_nevel,
                    child_table_va,
                    pgtype,
                    next_level_pte_indices,
                );
            }
        }
    }
}

/// Helper to print out page PTE
fn print_pte_page(indent: usize, pte_indices: PteIndices, pte: Entry) {
    println!(
        "{:indent$}[{:03}] Entry va:{:#018x} -> {:?} (pte:{:#016x})",
        "",
        pte_indices.last_index().unwrap_or(0),
        pte_indices.to_va(),
        pte,
        pte.0,
    );
}

/// Helper to print out table PTE
fn print_pte_table(indent: usize, i: usize, pte: Entry, table_va: usize) {
    println!(
        "{:indent$}[{:03}] Table va:{:#018x} {:?} (pte:{:#016x})",
        "", i, table_va, pte, pte.0,
    );
}

pub unsafe fn init_kernel_page_tables(
    page_table: &mut RootPageTable,
    dtb_range: PhysRange,
    available_mem: PhysRange,
) {
    pagealloc::init_page_allocator();

    // We use recursive page tables, but we have to be careful in the init call,
    // since the kpage_table is not currently pointed to by ttbr1_el1.  Any
    // recursive addressing of (511, 511, 511, 511) always points to the
    // physical address of the root page table, which isn't what we want here
    // because kpage_table hasn't been switched to yet.

    // Write the recursive entry
    unsafe {
        let entry = Entry::rw_kernel_data()
            .with_phys_addr(from_ptr_to_physaddr(page_table))
            .with_page_or_table(true);
        write_volatile(&mut page_table.entries[511], entry);
    }

    // TODO leave the first page unmapped to catch null pointer dereferences in unsafe code
    let custom_map = {
        // The DTB range might not end on a page boundary, so round up.
        let dtb_page_size = PageSize::Page4K;
        let dtb_range =
            PhysRange(dtb_range.start()..dtb_range.end().round_up(dtb_page_size.size() as u64));

        let text_range = boottext_range().add(&text_range());
        let ro_data_range = rodata_range();
        let data_range = data_range().add(&bss_range());
        let mmio_range = rpi_mmio().expect("mmio base detect failed");

        let mut map = [
            ("DTB", dtb_range, Entry::ro_kernel_data(), dtb_page_size),
            ("Kernel Text", text_range, Entry::ro_kernel_text(), PageSize::Page2M),
            ("Kernel RO Data", ro_data_range, Entry::ro_kernel_data(), PageSize::Page2M),
            ("Kernel Data", data_range, Entry::rw_kernel_data(), PageSize::Page2M),
            ("MMIO", mmio_range, Entry::rw_device(), PageSize::Page2M),
        ];
        map.sort_by_key(|a| a.1.start());
        map
    };

    println!("Memory map:");
    for (name, range, flags, page_size) in custom_map.iter() {
        let mapped_range = page_table
            .map_phys_range(
                name,
                range,
                VaMapping::Offset(KZERO),
                *flags,
                *page_size,
                RootPageTableType::Kernel,
            )
            .expect("error:init:mapping failed");

        println!(
            "  {:16}{} to {:#018x}..{:#018x} flags: {:?} page_size: {:?}",
            name, range, mapped_range.0, mapped_range.1, flags, page_size
        );
    }

    if let Err(err) = pagealloc::free_unused_ranges(&available_mem, custom_map.map(|m| m.1).iter())
    {
        panic!("error:Couldn't mark unused pages as free: err: {:?}", err);
    }
}

pub unsafe fn init_user_page_tables(page_table: &mut RootPageTable) {
    // Write the recursive entry
    unsafe {
        let entry = Entry::rw_kernel_data()
            .with_phys_addr(from_ptr_to_physaddr(page_table))
            .with_page_or_table(true);
        write_volatile(&mut page_table.entries[511], entry);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RootPageTableType {
    User,
    Kernel,
}

/// Return the root user-level page table physical address
fn ttbr0_el1() -> PhysAddr {
    #[cfg(not(test))]
    {
        let mut addr: u64;
        unsafe {
            core::arch::asm!("mrs {value}, ttbr0_el1", value = out(reg) addr);
        }
        PhysAddr::new(addr)
    }
    #[cfg(test)]
    PhysAddr::new(0)
}

/// Return the root kernel page table physical address
fn ttbr1_el1() -> PhysAddr {
    #[cfg(not(test))]
    {
        let mut addr: u64;
        unsafe {
            core::arch::asm!("mrs {value}, ttbr1_el1", value = out(reg) addr);
        }
        PhysAddr::new(addr)
    }
    #[cfg(test)]
    PhysAddr::new(0)
}

// TODO this should just call invalidate_all_tlb_entries afterwards?
#[allow(unused_variables)]
pub unsafe fn switch(page_table: &RootPageTable, pgtype: RootPageTableType) {
    #[cfg(not(test))]
    unsafe {
        let pt_phys = from_ptr_to_physaddr(page_table).addr();
        // https://forum.osdev.org/viewtopic.php?t=36412&p=303237
        match pgtype {
            RootPageTableType::User => {
                core::arch::asm!(
                    "msr ttbr0_el1, {pt_phys}",
                    "tlbi vmalle1is", // invalidate all TLB entries
                    "dsb ish",      // ensure write has completed
                    "isb",          // synchronize context and ensure that no instructions
                                    // are fetched using the old translation
                    pt_phys = in(reg) pt_phys);
            }
            RootPageTableType::Kernel => {
                core::arch::asm!(
                    "msr ttbr1_el1, {pt_phys}",
                    "tlbi vmalle1is", // invalidate all TLB entries
                    "dsb ish",      // ensure write has completed
                    "isb",          // synchronize context and ensure that no instructions
                                    // are fetched using the old translation
                    pt_phys = in(reg) pt_phys);
            }
        }
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
            "isb"             // synchronize context and ensure that no instructions
                              // are fetched using the old translation
        );
    }
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
        // assert_eq!(va_indices(0xffffffffffe00000), (256, 0, 36, 509));
        // assert_eq!(va_indices(0xfffffffffff00000), (256, 0, 36, 509));
        // assert_eq!(va_indices(0xffffffffe0000000), (256, 0, 36, 509));
        // assert_eq!(va_indices(0x1000), (0, 0, 0, 1));
    }

    #[test]
    fn test_recursive_table_addr() {
        assert_eq!(va_indices(0xffff800008000000), (256, 0, 64, 0));
        assert_eq!(
            va_indices(recursive_table_addr(
                RootPageTableType::Kernel,
                0xffff800008000000,
                Level::Level0
            )),
            (511, 511, 511, 511)
        );
        assert_eq!(
            va_indices(recursive_table_addr(
                RootPageTableType::Kernel,
                0xffff800008000000,
                Level::Level1
            )),
            (511, 511, 511, 256)
        );
        assert_eq!(
            va_indices(recursive_table_addr(
                RootPageTableType::Kernel,
                0xffff800008000000,
                Level::Level2
            )),
            (511, 511, 256, 0)
        );
        assert_eq!(
            va_indices(recursive_table_addr(
                RootPageTableType::Kernel,
                0xffff800008000000,
                Level::Level3
            )),
            (511, 256, 0, 64)
        );
        assert_eq!(
            va_indices(recursive_table_addr(
                RootPageTableType::Kernel,
                0xffff800008000000,
                Level::Level3
            )),
            (511, 256, 0, 64)
        );
    }

    #[test]
    fn test_pte_indices() {
        let p = PteIndices::none(RootPageTableType::User);
        assert_eq!(p, PteIndices::none(RootPageTableType::User));

        let p = p.with_next_index(1).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), None, None, None));

        let p = p.with_next_index(2).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), Some(2), None, None));

        let p = p.with_next_index(3).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), Some(2), Some(3), None));

        let p = p.with_next_index(4).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), Some(2), Some(3), Some(4)));

        let p = PteIndices::new(RootPageTableType::Kernel, Some(1), Some(2), None, None);
        let p = p.with_last_index(33).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::Kernel, Some(1), Some(33), None, None));
        assert_eq!(p.last_index(), Some(33));

        let p = PteIndices::new(RootPageTableType::Kernel, Some(1), Some(2), Some(3), Some(4));
        let p = p.with_last_index(100).unwrap();
        assert_eq!(
            p,
            PteIndices::new(RootPageTableType::Kernel, Some(1), Some(2), Some(3), Some(100))
        );
        assert_eq!(p.last_index(), Some(100));

        let p = PteIndices::new(RootPageTableType::Kernel, Some(15), Some(0), Some(400), Some(4));
        assert_eq!(va_indices(p.to_va()), (15, 0, 400, 4));

        let p = PteIndices::new(RootPageTableType::User, Some(0), Some(10), Some(40), Some(23));
        assert_eq!(va_indices(p.to_va()), (0, 10, 40, 23));
    }
}
