#![allow(non_upper_case_globals)]

/// Recursive page table implementation for aarch64.
/// Note that currently there are a lot of assumptions that we're dealing with
/// 4KiB tables here, although it supports various sizes of pages.
use crate::{
    kmem::{from_ptr_to_physaddr_offset_from_kzero, physaddr_as_ptr_mut_offset_from_kzero},
    pagealloc,
    param::KZERO,
};
use bitstruct::bitstruct;
use core::{fmt, ptr, sync::atomic::Ordering};
use core::{ptr::write_volatile, sync::atomic::AtomicUsize};
use num_enum::{FromPrimitive, IntoPrimitive};
use port::{
    mem::{PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, PhysAddr, PhysRange, VirtRange},
    pagealloc::PageAllocError,
};

#[cfg(not(test))]
use port::println;

//static mut KERNEL_PAGETABLE: RootPageTable = RootPageTable::empty();
static mut USER_PAGETABLE: RootPageTable = RootPageTable::empty();

pub fn kernel_pagetable() -> &'static mut RootPageTable {
    //unsafe { &mut *ptr::addr_of_mut!(KERNEL_PAGETABLE) }
    root_page_table(RootPageTableType::Kernel)
}

pub fn user_pagetable() -> &'static mut RootPageTable {
    unsafe { &mut *ptr::addr_of_mut!(USER_PAGETABLE) }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSize {
    Page4K,
    Page2M,
    Page1G,
}

impl PageSize {
    pub const fn size(&self) -> usize {
        match self {
            PageSize::Page4K => PAGE_SIZE_4K,
            PageSize::Page2M => PAGE_SIZE_2M,
            PageSize::Page1G => PAGE_SIZE_1G,
        }
    }
}

#[repr(C, align(4096))]
pub struct PhysPage4K([u8; PAGE_SIZE_4K]);

impl PhysPage4K {
    pub fn clear(&mut self) {
        unsafe {
            core::intrinsics::volatile_set_memory(&mut self.0, 0u8, 1);
        }
    }
}

#[repr(C, align(4096))]
pub struct VirtPage4K(pub [u8; PAGE_SIZE_4K]);

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
    pub struct Entry(pub u64) {
        pub valid: bool = 0;
        pub page_or_table: bool = 1;
        pub mair_index: Mair = 2..5;
        pub non_secure: bool = 5;
        pub access_permission: AccessPermission = 6..8;
        pub shareable: Shareable = 8..10;
        pub accessed: bool = 10; // Was accessed by code
        pub addr: u64 = 12..48;
        pub pxn: bool = 53; // Privileged eXecute Never
        pub uxn: bool = 54; // Unprivileged eXecute Never
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

    pub fn ro_kernel_data() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRo)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    pub fn ro_kernel_text() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRo)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(false)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    pub fn rw_device() -> Self {
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
            .with_uxn(false)
            .with_pxn(true)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    pub fn rw_user_data() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::AllRw)
            .with_shareable(Shareable::Inner)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Normal)
            .with_valid(true)
    }

    pub const fn with_phys_addr(self, pa: PhysAddr) -> Self {
        Entry(self.0).with_addr(pa.addr() >> 12)
    }

    pub fn phys_addr(self) -> PhysAddr {
        PhysAddr::new(self.addr() << 12)
    }

    pub fn is_table(self, level: Level) -> bool {
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
    MappingRecursiveIndex,
}

impl From<PageAllocError> for PageTableError {
    fn from(err: PageAllocError) -> PageTableError {
        PageTableError::AllocationFailed(err)
    }
}

#[repr(C, align(4096))]
pub struct Table {
    pub entries: [Entry; 512],
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
    fn next_mut<A: PageAllocator, B: VmTrait>(
        &mut self,
        page_allocator: &mut A,
        vmtrait_impl: &mut B,
        pgtype: RootPageTableType,
        level: Level,
        va: usize,
    ) -> Result<&mut Table, PageTableError> {
        // Try to get a valid page table entry.
        let index = va_index(va, level);
        let mut entry = self.entries[index];

        if entry.valid() && !entry.is_table(level) {
            println!("error:vm:next_mut:entry is not a valid table entry:{entry:?} {level:?}");
            return Err(PageTableError::EntryIsNotTable);
        }

        if entry.valid() {
            // Return the address of the next table
            let next_level = level.next().unwrap();
            return Ok(unsafe {
                &mut *vmtrait_impl.resolve_entry_mut::<Table>(entry, pgtype, va, next_level)
            });
        }

        // Create a new page table and write the entry into the parent table
        let page_pa = page_allocator.alloc_physpage();
        let page_pa = match page_pa {
            Ok(p) => p,
            Err(err) => {
                println!("error:vm:next_mut:can't allocate physpage");
                return Err(PageTableError::AllocationFailed(err));
            }
        };
        entry = Entry::rw_kernel_data().with_phys_addr(page_pa).with_page_or_table(true);
        vmtrait_impl.write_entry(&mut self.entries[index], entry);

        // Clear out the new page and return it as the next table
        let next_level = level.next().unwrap();
        let page = vmtrait_impl.resolve_entry_mut::<PhysPage4K>(entry, pgtype, va, next_level);
        return Ok(unsafe {
            (*page).clear();
            &mut *(page as *mut Table)
        });
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
    pub fn map(&self, pa: PhysAddr) -> usize {
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
    /// root_page_table should be a direct va - not a recursive va.
    fn map_to<A: PageAllocator, B: VmTrait>(
        &mut self,
        page_allocator: &mut A,
        vmtrait_impl: &mut B,
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
        let temp_recursive_entry = vmtrait_impl.generate_temp_recursive_address(self);
        let old_recursive_entry =
            vmtrait_impl.replace_recursive_entry(pgtype, temp_recursive_entry);

        let dest_entry = match page_size {
            PageSize::Page4K => self
                .next_mut(page_allocator, vmtrait_impl, pgtype, Level::Level0, va)
                .and_then(|t1| t1.next_mut(page_allocator, vmtrait_impl, pgtype, Level::Level1, va))
                .and_then(|t2| t2.next_mut(page_allocator, vmtrait_impl, pgtype, Level::Level2, va))
                .and_then(|t3| t3.entry_mut(Level::Level3, va)),
            PageSize::Page2M => self
                .next_mut(page_allocator, vmtrait_impl, pgtype, Level::Level0, va)
                .and_then(|t1| t1.next_mut(page_allocator, vmtrait_impl, pgtype, Level::Level1, va))
                .and_then(|t2| t2.entry_mut(Level::Level2, va)),
            PageSize::Page1G => self
                .next_mut(page_allocator, vmtrait_impl, pgtype, Level::Level0, va)
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

        vmtrait_impl.write_entry(dest_entry, entry);
        vmtrait_impl.write_recursive_entry(pgtype, old_recursive_entry);

        Ok(())
    }

    /// Map the physical range using the requested page size.
    /// This aligns on page size boundaries, and rounds the requested range so
    /// that both the alignment requirements are met and the requested range are
    /// covered.
    /// TODO Assuming some of these requests are dynamic, but should not fail,
    /// we should fall back to the smaller page sizes if the requested size fails.
    pub fn map_phys_range<A: PageAllocator, B: VmTrait>(
        &mut self,
        page_allocator: &mut A,
        vmtrait_impl: &mut B,
        debug_name: &str,
        range: &PhysRange,
        va_mapping: VaMapping,
        entry: Entry,
        page_size: PageSize,
        pgtype: RootPageTableType,
    ) -> Result<VirtRange, PageTableError> {
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
            self.map_to(
                page_allocator,
                vmtrait_impl,
                entry.with_phys_addr(pa),
                currva,
                page_size,
                pgtype,
            )?;
        }
        let mapped_virtrange = startva.map(|startva| VirtRange(startva..endva));
        mapped_virtrange.ok_or(PageTableError::PhysRangeIsZero)
    }
}

// TODO this needs to be a real virtual address allocator...
static next_free_device_page_va: AtomicUsize = AtomicUsize::new(KZERO + 0x100000000000);
pub fn next_free_device_page4k() -> VaMapping {
    next_free_device_page_va
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current + PageSize::Page4K.size())
        })
        .map(VaMapping::Addr)
        .expect("next_free_device_page4k: unable to return new va")
}

/// Return the root user or kernel level page table
pub fn root_page_table(pgtype: RootPageTableType) -> &'static mut RootPageTable {
    let page_table_pa = match pgtype {
        RootPageTableType::User => ttbr0_el1(),
        RootPageTableType::Kernel => ttbr1_el1(),
    };
    unsafe { &mut *physaddr_as_ptr_mut_offset_from_kzero::<RootPageTable>(page_table_pa) }
}

pub unsafe fn init_user_page_tables() {
    unsafe { init_empty_root_page_table(user_pagetable()) };
}

/// Given an empty, statically allocated page table.  We need to write a
/// recursive entry in the last entry.  To do this, we need to know the physical
/// address, but all we have is the virtual address
unsafe fn init_empty_root_page_table(root_page_table: &mut RootPageTable) {
    unsafe {
        let entry = Entry::rw_kernel_data()
            .with_phys_addr(from_ptr_to_physaddr_offset_from_kzero(root_page_table))
            .with_page_or_table(true);
        write_volatile(&mut root_page_table.entries[511], entry);
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
        let pt_phys = from_ptr_to_physaddr_offset_from_kzero(page_table).addr();
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

pub trait PageAllocator {
    fn alloc_physpage(&mut self) -> Result<PhysAddr, PageAllocError>;
}

#[derive(Clone, Copy)]
pub struct PhysPageAllocator {}

impl PageAllocator for PhysPageAllocator {
    fn alloc_physpage(&mut self) -> Result<PhysAddr, PageAllocError> {
        pagealloc::allocate_physpage()
    }
}

/// I guess we need a better name.  This is to wrap all things that would break if run in a
/// test, or if run before we have the MMU set up.
pub trait VmTrait {
    fn write_entry(&self, dest_entry: &mut Entry, entry: Entry);

    fn replace_recursive_entry(&mut self, pgtype: RootPageTableType, entry: Entry) -> Entry;

    fn write_recursive_entry(&mut self, pgtype: RootPageTableType, entry: Entry);

    fn generate_temp_recursive_address(&self, table: &Table) -> Entry;

    /// Resolve the next table or page from an entry.  In the kernel, this uses recursive
    /// page table addressing.  In tests, it uses the physical address directly
    /// from the entry (which is a valid virtual address in the test environment).
    fn resolve_entry_mut<T>(
        &self,
        entry: Entry,
        pgtype: RootPageTableType,
        va: usize,
        level: Level,
    ) -> *mut T;
}

#[derive(Clone, Copy)]
pub struct VmTraitImpl {}

impl VmTrait for VmTraitImpl {
    fn write_entry(&self, dest_entry: &mut Entry, entry: Entry) {
        unsafe {
            write_volatile(dest_entry, entry);
            invalidate_all_tlb_entries();
        }
    }

    fn replace_recursive_entry(&mut self, pgtype: RootPageTableType, entry: Entry) -> Entry {
        let page_table = root_page_table(pgtype);
        let old_recursive_entry = page_table.entries[511];
        self.write_recursive_entry(pgtype, entry);
        old_recursive_entry
    }

    fn write_recursive_entry(&mut self, pgtype: RootPageTableType, entry: Entry) {
        let page_table = root_page_table(pgtype);
        unsafe {
            // Return the recursive entry to its original state
            write_volatile(&mut page_table.entries[511], entry);
            // TODO Need to invalidate the single cache entry (+ optionally the recursive entry)
            invalidate_all_tlb_entries();
        }
    }

    fn generate_temp_recursive_address(&self, table: &Table) -> Entry {
        let pa = from_ptr_to_physaddr_offset_from_kzero(table);
        Entry::rw_kernel_data().with_phys_addr(pa).with_page_or_table(true)
    }

    fn resolve_entry_mut<T>(
        &self,
        _entry: Entry,
        pgtype: RootPageTableType,
        va: usize,
        level: Level,
    ) -> *mut T {
        recursive_table_addr(pgtype, va, level) as *mut T
    }
}

#[cfg(test)]
mod tests {
    use crate::vmdebug::va_indices;

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

    struct TestPageAllocator {
        free_pages: [PhysPage4K; 4],
        next_page_idx: usize,
    }

    impl TestPageAllocator {
        fn new() -> Self {
            Self {
                free_pages: core::array::from_fn(|_| PhysPage4K([0u8; PAGE_SIZE_4K])),
                next_page_idx: 0,
            }
        }
    }

    impl PageAllocator for TestPageAllocator {
        fn alloc_physpage(&mut self) -> Result<PhysAddr, PageAllocError> {
            if self.next_page_idx < self.free_pages.len() {
                let next_page = &self.free_pages[self.next_page_idx];
                self.next_page_idx += 1;
                Ok(PhysAddr::new(next_page as *const PhysPage4K as u64))
            } else {
                Err(PageAllocError::OutOfSpace)
            }
        }
    }

    struct TestVmTrait {
        root_page_table: RootPageTable,
    }

    impl TestVmTrait {
        fn new() -> Self {
            Self { root_page_table: RootPageTable::empty() }
        }
    }

    impl VmTrait for TestVmTrait {
        fn write_entry(&self, dest_entry: &mut Entry, entry: Entry) {
            *dest_entry = entry;
        }

        fn replace_recursive_entry(&mut self, _pgtype: RootPageTableType, entry: Entry) -> Entry {
            let old_entry = self.root_page_table.entries[511];
            self.root_page_table.entries[511] = entry;
            old_entry
        }

        fn write_recursive_entry(&mut self, _pgtype: RootPageTableType, entry: Entry) {
            self.root_page_table.entries[511] = entry;
        }

        fn generate_temp_recursive_address(&self, table: &Table) -> Entry {
            Entry::rw_kernel_data()
                .with_phys_addr(PhysAddr(table as *const _ as u64))
                .with_page_or_table(true)
        }

        fn resolve_entry_mut<T>(
            &self,
            entry: Entry,
            _pgtype: RootPageTableType,
            _va: usize,
            _level: Level,
        ) -> *mut T {
            // In tests, the "physical" address is actually a valid virtual address
            entry.phys_addr().addr() as *mut T
        }
    }

    #[test]
    fn map_phys_range_test() {
        let mut table = RootPageTable::empty();
        let mut test_page_allocator = TestPageAllocator::new();
        let mut test_vmtrait_impl = TestVmTrait::new();

        let mapped_virtrange = table
            .map_phys_range(
                &mut test_page_allocator,
                &mut test_vmtrait_impl,
                "test",
                &PhysRange::with_end(4096, 8 * 4096),
                VaMapping::Offset(KZERO),
                Entry::rw_kernel_data(),
                PageSize::Page4K,
                RootPageTableType::Kernel,
            )
            .expect("error:init:mapping failed");

        assert_eq!(mapped_virtrange, VirtRange::with_end(KZERO + 4096, KZERO + 8 * 4096));
    }
}
