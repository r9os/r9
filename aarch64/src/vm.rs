#![allow(non_upper_case_globals)]

use crate::{
    kalloc,
    kmem::{
        early_pagetables_addr, ebss_addr, eearly_pagetables_addr, eheap_addr, erodata_addr,
        etext_addr, heap_addr, text_addr, PhysAddr,
    },
    registers::rpi_mmio,
    Result,
};
use bitstruct::bitstruct;
use core::fmt;
use core::ptr::write_volatile;
use num_enum::{FromPrimitive, IntoPrimitive};

#[cfg(not(test))]
use port::println;

pub const PAGE_SIZE_4K: usize = 4 * 1024;
pub const PAGE_SIZE_2M: usize = 2 * 1024 * 1024;
pub const PAGE_SIZE_1G: usize = 1 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
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
    NonShareable = 0, // Non-shareable (single core)
    Unpredictable = 1,  // Unpredicatable!
    OuterShareable = 2, // Outer shareable (shared across CPUs, GPU)
    InnerShareable = 3, // Inner shareable (shared across CPUs)
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
        table: bool = 1;
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
    fn new(pa: PhysAddr) -> Self {
        Entry(0).with_addr(pa.addr() >> 12)
    }

    pub const fn empty() -> Entry {
        Entry(0)
    }

    fn rw_kernel_data() -> Self {
        Entry(0)
            .with_shareable(Shareable::InnerShareable)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Normal)
    }

    fn ro_kernel_data() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRo)
            .with_shareable(Shareable::InnerShareable)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Normal)
    }

    fn ro_kernel_text() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRw)
            .with_shareable(Shareable::InnerShareable)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(false)
            .with_mair_index(Mair::Normal)
    }

    fn ro_kernel_device() -> Self {
        Entry(0)
            .with_access_permission(AccessPermission::PrivRw)
            .with_shareable(Shareable::InnerShareable)
            .with_accessed(true)
            .with_uxn(true)
            .with_pxn(true)
            .with_mair_index(Mair::Device)
    }

    const fn with_phys_addr(self, pa: PhysAddr) -> Self {
        Entry(self.0).with_addr(pa.addr() >> 12)
    }

    fn phys_page_addr(self) -> PhysAddr {
        PhysAddr::new(self.addr() << 12)
    }

    fn virt_page_addr(self) -> usize {
        self.phys_page_addr().to_virt()
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
        if self.table() {
            write!(f, " Table")?;
        } else {
            write!(f, " Page")?;
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
#[derive(Debug, Clone, Copy)]
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

#[repr(C, align(4096))]
pub struct Table {
    entries: [Entry; 512],
}

impl Table {
    fn index(level: Level, va: usize) -> usize {
        match level {
            Level::Level0 => (va >> 39) & 0x1FF,
            Level::Level1 => (va >> 30) & 0x1FF,
            Level::Level2 => (va >> 21) & 0x1FF,
            Level::Level3 => (va >> 12) & 0x1FF,
        }
    }

    pub fn entry_mut(&mut self, level: Level, va: usize) -> Option<&mut Entry> {
        Some(&mut self.entries[Self::index(level, va)])
    }

    fn child_table(&self, entry: Entry) -> Option<&Table> {
        if !entry.valid() {
            return None;
        }
        let raw_ptr = entry.virt_page_addr();
        Some(unsafe { &*(raw_ptr as *const Table) })
    }

    fn next(&self, level: Level, va: usize) -> Option<&Table> {
        let idx = Self::index(level, va);
        let entry = self.entries[idx];
        self.child_table(entry)
    }

    fn next_mut(&mut self, level: Level, va: usize) -> Option<&mut Table> {
        let index = Self::index(level, va);
        let mut entry = self.entries[index];
        // println!("next_mut(level:{:?}, va:{:016x}, index:{}): entry:{:?}", level, va, index, entry);
        if !entry.valid() {
            let page = kalloc::alloc()?;
            page.clear();
            entry = Entry::new(PhysAddr::from_ptr(page)).with_valid(true).with_table(true);
            unsafe {
                write_volatile(&mut self.entries[index], entry);
            }
        }
        let raw_ptr = entry.virt_page_addr();
        let next_table = unsafe { &mut *(raw_ptr as *mut Table) };
        Some(next_table)
    }
}

pub type PageTable = Table;

impl PageTable {
    pub const fn empty() -> PageTable {
        PageTable { entries: [Entry::empty(); 512] }
    }

    pub fn map_to(&mut self, entry: Entry, va: usize, page_size: PageSize) -> Result<()> {
        // println!("map_to(entry: {:?}, va: {:#x}, page_size {:?})", entry, va, page_size);
        let old_entry = match page_size {
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

        if let Some(old_entry) = old_entry {
            let entry = entry.with_valid(true);
            // println!("Some {:?}, New {:?}", old_entry, entry);
            // println!("{:p}", old_entry);
            unsafe {
                write_volatile(old_entry, entry);
            }
            return Ok(());
        }
        Err("Allocation failed")
    }

    pub fn map_phys_range(
        &mut self,
        start: PhysAddr,
        end: PhysAddr,
        entry: Entry,
        page_size: PageSize,
    ) -> Result<()> {
        for pa in PhysAddr::step_by_rounded(start, end, page_size.size()) {
            self.map_to(entry.with_phys_addr(pa), pa.to_virt(), page_size)?;
        }
        Ok(())
    }

    /// Recursively write out the table and all its children
    pub fn print_tables(&self) {
        println!("Root  va:{:p}", self);
        self.print_table_at_level(Level::Level0);
    }

    /// Recursively write out the table and all its children
    fn print_table_at_level(&self, level: Level) {
        let indent = 2 + level.depth() * 2;
        for (i, &pte) in self.entries.iter().enumerate() {
            if pte.valid() {
                print_pte(indent, i, pte);

                if pte.table() {
                    if let Some(child_table) = self.child_table(pte) {
                        child_table.print_table_at_level(level.next().unwrap());
                    }
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
fn print_pte(indent: usize, i: usize, pte: Entry) {
    println!(
        "{:indent$}[{:03}] va:{:#016x} -> pa:({:?}) (pte:{:#016x})",
        "",
        i,
        pte.virt_page_addr(),
        pte,
        pte.0,
    );
}

pub unsafe fn init(kpage_table: &mut PageTable, dtb_phys: PhysAddr, edtb_phys: PhysAddr) {
    //use PageFlags as PF;

    let text_phys = PhysAddr::from_virt(text_addr());
    let etext_phys = PhysAddr::from_virt(etext_addr());
    let erodata_phys = PhysAddr::from_virt(erodata_addr());
    let ebss_phys = PhysAddr::from_virt(ebss_addr());
    let heap_phys = PhysAddr::from_virt(heap_addr());
    let eheap_phys = PhysAddr::from_virt(eheap_addr());
    let early_pagetables_phys = PhysAddr::from_virt(early_pagetables_addr());
    let eearly_pagetables_phys = PhysAddr::from_virt(eearly_pagetables_addr());

    let mmio = rpi_mmio().expect("mmio base detect failed");
    let mmio_end = PhysAddr::from(mmio + (2 * PAGE_SIZE_2M as u64));

    let custom_map = [
        // TODO We don't actualy unmap the first page...  We should to achieve:
        // Note that the first page is left unmapped to try and
        // catch null pointer dereferences in unsafe code: defense
        // in depth!

        // DTB
        (dtb_phys, edtb_phys, Entry::ro_kernel_data(), PageSize::Page4K),
        // Kernel text
        (text_phys, etext_phys, Entry::ro_kernel_text(), PageSize::Page2M),
        // Kernel read-only data
        (etext_phys, erodata_phys, Entry::ro_kernel_data(), PageSize::Page2M),
        // Kernel BSS
        (erodata_phys, ebss_phys, Entry::rw_kernel_data(), PageSize::Page2M),
        // Kernel heap
        (heap_phys, eheap_phys, Entry::rw_kernel_data(), PageSize::Page2M),
        // Page tables
        (early_pagetables_phys, eearly_pagetables_phys, Entry::rw_kernel_data(), PageSize::Page2M),
        // MMIO
        (mmio, mmio_end, Entry::ro_kernel_device(), PageSize::Page2M),
    ];

    for (start, end, flags, page_size) in custom_map.iter() {
        kpage_table.map_phys_range(*start, *end, *flags, *page_size).expect("init mapping failed");
    }
}

/// Return the root kernel page table physical address
fn ttbr1_el1() -> u64 {
    let mut addr: u64;
    unsafe {
        core::arch::asm!("mrs {value}, ttbr1_el1", value = out(reg) addr);
    }
    addr
}

pub unsafe fn switch(kpage_table: &PageTable) {
    #[cfg(not(test))]
    unsafe {
        let pt_phys = PhysAddr::from_ptr(kpage_table).addr();
        core::arch::asm!(
            "msr ttbr1_el1, {pt_phys}",
            "dsb ish",
            "isb",
            pt_phys = in(reg) pt_phys);
    }
}

/// Return the root kernel page table
pub fn kernel_root() -> &'static mut PageTable {
    unsafe {
        let ttbr1_el1 = ttbr1_el1();
        &mut *PhysAddr::new(ttbr1_el1).to_ptr_mut::<PageTable>()
    }
}
