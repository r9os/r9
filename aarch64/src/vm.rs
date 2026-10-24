#![allow(non_upper_case_globals)]

/// Recursive page table implementation for aarch64.
/// Note that currently there are a lot of assumptions that we're dealing with
/// 4KiB tables here, although it supports various sizes of pages.
use crate::{
    kmap::kmap,
    kmem::paddr,
    pagealloc,
    param::{KVA_BASE, KZERO, UVA_END},
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

/// The user root that's loaded when there's no user address space.  It holds
/// only its recursive entry, so every other user address faults, including
/// the null page.
static mut EMPTY_USER_ROOT: RootPageTable = RootPageTable::empty();

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

    /// An entry that points to the next level's table at `pa`.  A walk ignores
    /// table entry bits [11:2] and [58:52], so the page attributes set here
    /// only apply when the recursive window reads the entry as a page.  They
    /// map the table kernel read-write, never executable and inner-shareable.
    pub fn table(pa: PhysAddr) -> Self {
        Self::rw_kernel_data().with_phys_addr(pa).with_page_or_table(true)
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

    pub fn ro_user_text() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::AllRo)
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
    // The table indices: VA bits [VA_BITS-1:12].
    let indices_mask = (UVA_END - 1) & !(PAGE_SIZE_4K - 1);
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
        RootPageTableType::Kernel => KVA_BASE,
        RootPageTableType::User => 0,
    };
    msbits | recursive_indices | ((indices >> shift) & indices_mask)
}

/// Return the level 3 entry that maps `va` in the kernel page table, reached
/// through the kernel's recursive entry.  The level 3 table for `va` must
/// already exist, as vminit builds for the kmap and kernel stack regions.
fn kernel_pte(va: usize) -> *mut Entry {
    let table = recursive_table_addr(RootPageTableType::Kernel, va, Level::Level3) as *mut Entry;
    table.wrapping_add(va_index(va, Level::Level3))
}

/// Map the physical page at `pa` at the kernel address `va`, kernel read-write
/// and never executable.
///
/// # Safety
///
/// The level 3 table for `va` must exist, as vminit builds for the kmap and
/// kernel stack regions, and `va`'s entry must be invalid.  `pa` must be RAM.
pub unsafe fn map_kernel_page(va: usize, pa: PhysAddr) {
    let entry = Entry::rw_kernel_data().with_phys_addr(pa).with_page_or_table(true);
    // SAFETY: the caller guarantees the table exists and the entry is free.
    unsafe { write_volatile(kernel_pte(va), entry) };
    // The entry was invalid, so no TLB entry for it can exist: the TLB doesn't
    // hold faulting translations.  Make the entry visible to the table walker
    // before va is used.
    #[cfg(not(test))]
    // SAFETY: barriers only.
    unsafe {
        core::arch::asm!("dsb ishst", "isb");
    }
}

/// Unmap the kernel page at `va`, and return the physical page that was mapped
/// there.
///
/// # Safety
///
/// `va` must have been mapped with map_kernel_page, and nothing may still use
/// it.
pub unsafe fn unmap_kernel_page(va: usize) -> PhysAddr {
    let pte = kernel_pte(va);
    // SAFETY: the caller guarantees the entry is one map_kernel_page wrote.
    let pa = unsafe { (*pte).phys_addr() };
    unsafe { write_volatile(pte, Entry::empty()) };
    invalidate_kernel_va(va);
    pa
}

/// Remove any TLB entry for the kernel address `va` on every CPU.
#[allow(unused_variables)]
fn invalidate_kernel_va(va: usize) {
    #[cfg(not(test))]
    // SAFETY: TLB maintenance and barriers only.
    unsafe {
        // Make the cleared entry visible to the table walker, then remove the
        // old translation.  Kernel entries are global, so invalidate for all
        // ASIDs (vaae1is).  The operand's bits [43:0] are VA[55:12]; the bits
        // above must be clear, as bits [47:44] are the TTL hint (Arm ARM,
        // TLBI VAAE1IS; Linux __TLBI_VADDR masks the same way).
        core::arch::asm!(
            "dsb ishst",
            "tlbi vaae1is, {page}",
            "dsb ish",
            "isb",
            page = in(reg) (va >> 12) & ((1 << 44) - 1),
        );
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum PageTableError {
    AllocationFailed(PageAllocError),
    EntryIsNotTable,
    PhysRangeIsZero,
    PhysRangeIsNotOnPageBoundary,
    MappingRecursiveIndex,
    NoAddressSpace,
    EmptyAddressSpace,
    AlreadyMapped,
    VaOutOfRange,
}

/// Check that `va` can be mapped in a `pgtype` page table: it must be in the
/// half of the address space that the table translates, and not in the range
/// covered by the recursive entry.
fn check_mappable_va(va: usize, pgtype: RootPageTableType) -> Result<(), PageTableError> {
    let in_range = match pgtype {
        RootPageTableType::User => va < UVA_END,
        RootPageTableType::Kernel => va >= KVA_BASE,
    };
    if !in_range {
        return Err(PageTableError::VaOutOfRange);
    }
    if va_index(va, Level::Level0) == 511 {
        return Err(PageTableError::MappingRecursiveIndex);
    }
    Ok(())
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

        // Add a new, empty table to this table.  The page is zeroed before it's
        // linked, so the table walker never sees stale entries in it, even
        // speculatively.
        let page_pa = page_allocator.alloc_physpage();
        let page_pa = match page_pa {
            Ok(p) => p,
            Err(err) => {
                println!("error:vm:next_mut:can't allocate physpage");
                return Err(PageTableError::AllocationFailed(err));
            }
        };
        entry = Entry::table(page_pa);
        vmtrait_impl.write_entry(&mut self.entries[index], entry);

        let next_level = level.next().unwrap();
        Ok(unsafe { &mut *vmtrait_impl.resolve_entry_mut::<Table>(entry, pgtype, va, next_level) })
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

    /// Map va to entry, creating any intermediate page tables that don't
    /// already exist.  If va is already mapped, fail with AlreadyMapped:
    /// replacing a valid entry would need break-before-make, and would leak
    /// the page it mapped.  self must be the active root of its half, reached
    /// through the recursive window, since that's how the tables below it are
    /// reached.
    fn map_to<A: PageAllocator, B: VmTrait>(
        &mut self,
        page_allocator: &mut A,
        vmtrait_impl: &mut B,
        entry: Entry,
        va: usize,
        page_size: PageSize,
        pgtype: RootPageTableType,
    ) -> Result<(), PageTableError> {
        if let Err(err) = check_mappable_va(va, pgtype) {
            println!("error:vm:map_to:can't map va:{va:#x} pgtype:{pgtype:?} err:{err:?}");
            return Err(err);
        }

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

        if dest_entry.valid() {
            println!("error:vm:map_to:va:{va:#x} is already mapped");
            return Err(PageTableError::AlreadyMapped);
        }

        // Entries at level 3 should have the page flag set
        let entry =
            if page_size == PageSize::Page4K { entry.with_page_or_table(true) } else { entry };

        vmtrait_impl.write_entry(dest_entry, entry);

        Ok(())
    }

    /// Map the physical range using the requested page size.
    /// This aligns on page size boundaries, and rounds the requested range so
    /// that both the alignment requirements are met and the requested range are
    /// covered.
    /// TODO Assuming some of these requests are dynamic, but should not fail,
    /// we should fall back to the smaller page sizes if the requested size fails.
    fn map_range<A: PageAllocator, B: VmTrait>(
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
        if !range.start.is_multiple_of(page_size.size() as u64)
            || !range.end.is_multiple_of(page_size.size() as u64)
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
        let mapped_virtrange = startva.map(|startva| VirtRange::new(startva, endva));
        mapped_virtrange.ok_or(PageTableError::PhysRangeIsZero)
    }
}

/// Map the physical range using the requested page size, in the active root
/// of `pgtype`'s half.  Only the active tables are ever edited, through the
/// recursive window, so there's no table argument: to map into a user
/// address space, switch to it first.
pub fn map_phys_range<A: PageAllocator, B: VmTrait>(
    page_allocator: &mut A,
    vmtrait_impl: &mut B,
    debug_name: &str,
    range: &PhysRange,
    va_mapping: VaMapping,
    entry: Entry,
    page_size: PageSize,
    pgtype: RootPageTableType,
) -> Result<VirtRange, PageTableError> {
    let root = match vmtrait_impl.active_root(pgtype) {
        Ok(root) => root,
        Err(err) => {
            println!("error:vm:map_phys_range:{debug_name}: no active {pgtype:?} root: {err:?}");
            return Err(err);
        }
    };
    // SAFETY: root is the active root of its half, reached through the
    // recursive window.  While mapping, the only other path into this half's
    // tables is kmap, which zeroes new tables and writes only the kmap
    // region's level 3 table.  Nothing maps into the kmap region through
    // here, so no reference made below aliases what kmap writes.
    let root = unsafe { &mut *root };
    root.map_range(
        page_allocator,
        vmtrait_impl,
        debug_name,
        range,
        va_mapping,
        entry,
        page_size,
        pgtype,
    )
}

// TODO this needs to be a real virtual address allocator...
static next_free_device_page_va: AtomicUsize = AtomicUsize::new(KZERO + 0x100000000000);
pub fn next_free_device_page4k() -> VaMapping {
    next_free_device_page_va
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current + PageSize::Page4K.size())
        })
        .map(VaMapping::Addr)
        .expect("next_free_device_page4k: unable to return new va")
}

/// Write the empty user root's recursive entry, once, at boot.
///
/// # Safety
///
/// It must be called before the empty root is first loaded, and no other CPU
/// may be running.
pub unsafe fn init_empty_user_root() {
    let root = unsafe { &mut *ptr::addr_of_mut!(EMPTY_USER_ROOT) };
    unsafe { init_empty_root_page_table(root) };
}

/// A user address space: the physical address of its root page table.  The
/// root isn't mapped at KZERO, so before it's loaded it's only reached through
/// kmap, and after that through the recursive window.  Dropping it without
/// release leaks its page tables.
pub struct AddressSpace {
    root: PhysAddr,
}

impl AddressSpace {
    /// The empty address space, which is loaded when there's no other.  Its
    /// root, a static, holds only its recursive entry, so every user address
    /// faults, including null.  It can't be released.
    pub fn empty() -> Self {
        Self { root: paddr(ptr::addr_of!(EMPTY_USER_ROOT)) }
    }

    /// Allocate a new address space, with nothing mapped.
    pub fn new() -> Result<Self, PageAllocError> {
        let root = pagealloc::allocate_zeroed_physpage()?;
        let k = kmap(root);
        // SAFETY: kmaps the newly allocated root, which nothing else refers
        // to yet.
        unsafe { write_volatile((k.va() as *mut Entry).add(511), Entry::table(root)) };
        Ok(Self { root })
    }
}

/// Release `space`: free its page tables, and pass each page or block mapped in
/// it to `free_leaf`, which frees it if the caller owns it.  Not every page
/// mapped in an address space belongs to it, so the caller decides, as Plan
/// 9's mmurelease frees only the MMU's pages and segments free their own.
///
/// If `space` is loaded, the empty address space is loaded first.  Its switch
/// flushes the TLB, and nothing can then walk `space`'s tables, so they're
/// freed through kmap without invalidating anything.  The empty address space
/// can't be released.
///
/// # Safety
///
/// Nothing may still refer to memory through `space`, and `space` may not be
/// loaded on another CPU.
pub unsafe fn release(
    space: AddressSpace,
    mut free_leaf: impl FnMut(PhysAddr),
) -> Result<(), PageTableError> {
    if space.root == AddressSpace::empty().root {
        return Err(PageTableError::EmptyAddressSpace);
    }
    if ttbr0_el1() == space.root {
        // SAFETY: the caller guarantees nothing uses space any more.
        unsafe { switch(&AddressSpace::empty()) };
    }
    // If space wasn't loaded, nothing in the TLB can refer to it: without
    // ASIDs, every switch flushes the whole TLB.  Once there are ASIDs,
    // release must also invalidate space's ASID here.
    free_tables(&VmTraitImpl {}, space.root, Level::Level0, &mut free_leaf, &mut |pa| {
        port::pagealloc::free_physpage(pa).expect("vm: couldn't free a page table")
    });
    Ok(())
}

/// Walk the table at `table_pa`, at `level`, passing each page or block it
/// maps to `free_leaf`, and each table, this one last, to `free_table`.  The
/// root's entry 511 is its recursive entry, so it's skipped.
fn free_tables<B: VmTrait>(
    vmtrait_impl: &B,
    table_pa: PhysAddr,
    level: Level,
    free_leaf: &mut impl FnMut(PhysAddr),
    free_table: &mut impl FnMut(PhysAddr),
) {
    vmtrait_impl.with_table(table_pa, |table| {
        for (i, entry) in table.entries.iter().enumerate() {
            if !entry.valid() || (level == Level::Level0 && i == 511) {
                continue;
            }
            if entry.is_table(level) {
                let next = level.next().unwrap();
                free_tables(vmtrait_impl, entry.phys_addr(), next, free_leaf, free_table);
            } else {
                free_leaf(entry.phys_addr());
            }
        }
    });
    free_table(table_pa);
}

/// Load `space` into ttbr0_el1, making it the user address space that
/// map_phys_range edits.
///
/// # Safety
///
/// Nothing may still refer to memory through the user address space that's
/// being replaced.
pub unsafe fn switch(space: &AddressSpace) {
    unsafe { load_user_root(space.root) };
}

/// Given an empty, statically allocated page table.  We need to write a
/// recursive entry in the last entry.  To do this, we need to know the physical
/// address, but all we have is the virtual address
unsafe fn init_empty_root_page_table(root_page_table: &mut RootPageTable) {
    unsafe {
        let entry = Entry::table(paddr(root_page_table));
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

/// Load the user root at `root` into ttbr0_el1.
#[allow(unused_variables)]
unsafe fn load_user_root(root: PhysAddr) {
    // Make the new root's entries visible to the table walker before it's
    // loaded.
    write_barrier();
    #[cfg(not(test))]
    unsafe {
        // https://forum.osdev.org/viewtopic.php?t=36412&p=303237
        core::arch::asm!(
            "msr ttbr0_el1, {root}",
            "tlbi vmalle1is", // invalidate all TLB entries
            "dsb ish",        // ensure write has completed
            "isb",            // synchronize context and ensure that no instructions
                              // are fetched using the old translation
            root = in(reg) root.addr());
    }
}

/// Wait for earlier stores, including to page tables, to be visible to every
/// CPU and table walker in the inner-shareable domain.  Table walks are
/// cacheable and inner-shareable (TCR_EL1's IRGN, ORGN and SH), so no cache
/// maintenance is needed.
fn write_barrier() {
    #[cfg(not(test))]
    // SAFETY: a barrier only.
    unsafe {
        core::arch::asm!("dsb ishst");
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
    /// Allocate a physical page, which must already be zeroed: it may become a
    /// page table as soon as it's returned.
    fn alloc_physpage(&mut self) -> Result<PhysAddr, PageAllocError>;
}

#[derive(Clone, Copy)]
pub struct PhysPageAllocator {}

impl PageAllocator for PhysPageAllocator {
    fn alloc_physpage(&mut self) -> Result<PhysAddr, PageAllocError> {
        pagealloc::allocate_zeroed_physpage()
    }
}

/// I guess we need a better name.  This is to wrap all things that would break if run in a
/// test, or if run before we have the MMU set up.
pub trait VmTrait {
    fn write_entry(&self, dest_entry: &mut Entry, entry: Entry);

    /// Return the active root of `pgtype`'s half.  In the kernel, this is its
    /// address in the recursive window.
    fn active_root(&self, pgtype: RootPageTableType) -> Result<*mut RootPageTable, PageTableError>;

    /// Run `f` on the page table at `pa`, which isn't active.  In the kernel,
    /// it's reached through kmap.
    fn with_table<R>(&self, pa: PhysAddr, f: impl FnOnce(&Table) -> R) -> R;

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
            // Make earlier writes, such as zeroing a new table, visible to the
            // table walker before the entry that refers to them.
            write_barrier();
            write_volatile(dest_entry, entry);
            // Make the entry visible to the table walker before the TLB
            // invalidate, so the old translation can't be cached again.
            write_barrier();
            invalidate_all_tlb_entries();
        }
    }

    fn active_root(&self, pgtype: RootPageTableType) -> Result<*mut RootPageTable, PageTableError> {
        if pgtype == RootPageTableType::User && ttbr0_el1() == AddressSpace::empty().root {
            return Err(PageTableError::NoAddressSpace);
        }
        // Every index of the root's own window address is 511.
        Ok(recursive_table_addr(pgtype, 0, Level::Level0) as *mut RootPageTable)
    }

    fn with_table<R>(&self, pa: PhysAddr, f: impl FnOnce(&Table) -> R) -> R {
        let k = kmap(pa);
        // SAFETY: kmaps the whole of a page table that isn't active, which
        // nothing else refers to while f runs.
        f(unsafe { &*(k.va() as *const Table) })
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
    use crate::param::{UADDRTOP, UTZERO};
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

    /// The test's roots.  A user root of None means there's no user address
    /// space.
    struct TestVmTrait {
        kernel: *mut RootPageTable,
        user: Option<*mut RootPageTable>,
    }

    impl TestVmTrait {
        /// Use `root` as the active root of both halves.
        fn new(root: &mut RootPageTable) -> Self {
            let root = root as *mut RootPageTable;
            Self { kernel: root, user: Some(root) }
        }
    }

    impl VmTrait for TestVmTrait {
        fn write_entry(&self, dest_entry: &mut Entry, entry: Entry) {
            *dest_entry = entry;
        }

        fn with_table<R>(&self, pa: PhysAddr, f: impl FnOnce(&Table) -> R) -> R {
            // In tests, the "physical" address is actually a valid virtual address
            f(unsafe { &*(pa.addr() as *const Table) })
        }

        fn active_root(
            &self,
            pgtype: RootPageTableType,
        ) -> Result<*mut RootPageTable, PageTableError> {
            match pgtype {
                RootPageTableType::Kernel => Ok(self.kernel),
                RootPageTableType::User => self.user.ok_or(PageTableError::NoAddressSpace),
            }
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
    fn cant_map_recursive_index() {
        let mut table = RootPageTable::empty();
        let mut test_page_allocator = TestPageAllocator::new();
        let mut test_vmtrait_impl = TestVmTrait::new(&mut table);

        let result = map_phys_range(
            &mut test_page_allocator,
            &mut test_vmtrait_impl,
            "recursive",
            &PhysRange::with_len(0x20_0000, PAGE_SIZE_4K),
            VaMapping::Addr(UADDRTOP),
            Entry::rw_user_data(),
            PageSize::Page4K,
            RootPageTableType::User,
        );
        assert!(matches!(result, Err(PageTableError::MappingRecursiveIndex)));
    }

    #[test]
    fn cant_map_va_outside_table_range() {
        let mut table = RootPageTable::empty();
        let mut test_page_allocator = TestPageAllocator::new();
        let mut test_vmtrait_impl = TestVmTrait::new(&mut table);

        // Above the 48-bit user range, and a kernel address in a user table,
        // would otherwise be truncated to UTZERO.
        for (va, pgtype) in [
            (UVA_END | UTZERO, RootPageTableType::User),
            (KZERO | UTZERO, RootPageTableType::User),
            (UTZERO, RootPageTableType::Kernel),
        ] {
            let result = map_phys_range(
                &mut test_page_allocator,
                &mut test_vmtrait_impl,
                "outofrange",
                &PhysRange::with_len(0x20_0000, PAGE_SIZE_4K),
                VaMapping::Addr(va),
                Entry::rw_user_data(),
                PageSize::Page4K,
                pgtype,
            );
            assert!(matches!(result, Err(PageTableError::VaOutOfRange)), "va:{va:#x}");
        }
    }

    #[test]
    fn map_phys_range_test() {
        let mut table = RootPageTable::empty();
        let mut test_page_allocator = TestPageAllocator::new();
        let mut test_vmtrait_impl = TestVmTrait::new(&mut table);

        let mapped_virtrange = map_phys_range(
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

        assert_eq!(mapped_virtrange, VirtRange::new(KZERO + 4096, KZERO + 8 * 4096));
    }

    #[test]
    fn table_entry_attributes() {
        let pa = PhysAddr::new(0x40_0000);
        let e = Entry::table(pa);
        assert!(e.valid() && e.page_or_table());
        assert_eq!(e.phys_addr(), pa);
        // As a page, through the recursive window: kernel RW, never
        // executable, inner-shareable, normal memory, accessed.
        assert_eq!(u8::from(e.access_permission()), u8::from(AccessPermission::PrivRw));
        assert!(e.pxn() && e.uxn() && e.accessed());
        assert_eq!(u8::from(e.shareable()), u8::from(Shareable::Inner));
        assert_eq!(u8::from(e.mair_index()), u8::from(Mair::Normal));
    }

    #[test]
    fn cant_map_user_pages_without_an_address_space() {
        let mut kernel = RootPageTable::empty();
        let mut test_page_allocator = TestPageAllocator::new();
        let mut test_vmtrait_impl = TestVmTrait { kernel: &mut kernel, user: None };

        let result = map_phys_range(
            &mut test_page_allocator,
            &mut test_vmtrait_impl,
            "nospace",
            &PhysRange::with_len(0x20_0000, PAGE_SIZE_4K),
            VaMapping::Addr(UTZERO),
            Entry::rw_user_data(),
            PageSize::Page4K,
            RootPageTableType::User,
        );
        assert!(matches!(result, Err(PageTableError::NoAddressSpace)));
        // Nothing was allocated or mapped.
        assert_eq!(test_page_allocator.next_page_idx, 0);
    }

    #[test]
    fn cant_map_an_address_twice() {
        let mut table = RootPageTable::empty();
        let mut test_page_allocator = TestPageAllocator::new();
        let mut test_vmtrait_impl = TestVmTrait::new(&mut table);

        for (pa, expected_ok) in [(0x20_0000, true), (0x40_0000, false)] {
            let result = map_phys_range(
                &mut test_page_allocator,
                &mut test_vmtrait_impl,
                "twice",
                &PhysRange::with_len(pa, PAGE_SIZE_4K),
                VaMapping::Addr(UTZERO),
                Entry::rw_user_data(),
                PageSize::Page4K,
                RootPageTableType::User,
            );
            if expected_ok {
                assert!(result.is_ok());
            } else {
                assert!(matches!(result, Err(PageTableError::AlreadyMapped)));
            }
        }
    }

    #[test]
    fn freeing_tables_finds_every_table_and_leaf() {
        let mut root = RootPageTable::empty();
        let root_pa = PhysAddr::new(&raw const root as u64);
        let mut test_page_allocator = TestPageAllocator::new();
        let mut test_vmtrait_impl = TestVmTrait::new(&mut root);

        // Two pages that share their level 1, 2 and 3 tables.
        for (i, pa) in [0x20_0000, 0x40_0000].into_iter().enumerate() {
            map_phys_range(
                &mut test_page_allocator,
                &mut test_vmtrait_impl,
                "release",
                &PhysRange::with_len(pa, PAGE_SIZE_4K),
                VaMapping::Addr(UTZERO + i * PAGE_SIZE_4K),
                Entry::rw_user_data(),
                PageSize::Page4K,
                RootPageTableType::User,
            )
            .expect("couldn't map");
        }

        let mut leaves = Vec::new();
        let mut tables = Vec::new();
        free_tables(
            &test_vmtrait_impl,
            root_pa,
            Level::Level0,
            &mut |pa| leaves.push(pa),
            &mut |pa| tables.push(pa),
        );

        assert_eq!(leaves, [PhysAddr::new(0x20_0000), PhysAddr::new(0x40_0000)]);
        // The three allocated tables, deepest first, then the root.
        let allocated: Vec<PhysAddr> = test_page_allocator.free_pages[..3]
            .iter()
            .rev()
            .map(|p| PhysAddr::new(p as *const PhysPage4K as u64))
            .collect();
        assert_eq!(test_page_allocator.next_page_idx, 3);
        assert_eq!(tables[..3], allocated[..]);
        assert_eq!(tables[3], root_pa);
    }
}
