use crate::{
    address::{PhysicalAddress, VirtualAddress},
    memory::kalloc,
};

/// a single PageTable with 512 entries
#[repr(C)]
pub struct PageTable {
    pub entries: [PageTableEntry; 512],
}

static mut KERNEL_PAGETABLE: PageTable = PageTable::empty();

#[allow(dead_code)]
impl PageTable {
    pub fn as_addr(&self) -> usize {
        self.entries.as_ptr() as usize
    }

    pub const fn empty() -> PageTable {
        Self { entries: [PageTableEntry { entry: 0 }; 512] }
    }

    pub fn len() -> usize {
        512
    }
}

#[allow(dead_code)]
#[repr(usize)]
#[derive(Copy, Clone)]
pub enum EntryBits {
    None = 0,
    Valid = 1 << 0,
    Read = 1 << 1,
    Write = 1 << 2,
    Execute = 1 << 3,
    User = 1 << 4,
    Global = 1 << 5,
    Access = 1 << 6,
    Dirty = 1 << 7,

    // Convenience combinations
    ReadWrite = 1 << 1 | 1 << 2,
    ReadExecute = 1 << 1 | 1 << 3,
    ReadWriteExecute = 1 << 1 | 1 << 2 | 1 << 3,

    // User Convenience Combinations
    UserReadWrite = 1 << 1 | 1 << 2 | 1 << 4,
    UserReadExecute = 1 << 1 | 1 << 3 | 1 << 4,
    UserReadWriteExecute = 1 << 1 | 1 << 2 | 1 << 3 | 1 << 4,
}

impl EntryBits {
    pub fn val(self) -> usize {
        self as usize
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct PageTableEntry {
    pub entry: usize,
}

#[allow(dead_code)]
impl PageTableEntry {
    pub fn is_valid(&self) -> bool {
        self.get_entry() & EntryBits::Valid.val() != 0
    }

    // The first bit (bit index #0) is the V bit for
    // valid.
    pub fn is_invalid(&self) -> bool {
        !self.is_valid()
    }

    // A leaf has one or more RWX bits set
    pub fn is_leaf(&self) -> bool {
        self.get_entry() & 0xe != 0
    }

    pub fn is_branch(&self) -> bool {
        !self.is_leaf()
    }

    pub fn set_entry(&mut self, entry: usize) {
        self.entry = entry;
    }

    pub fn get_entry(&self) -> usize {
        self.entry
    }
}

/// Map a virtual address to a physical address using 4096-byte page
/// size.
/// root: a mutable reference to the root Table
/// vaddr: The virtual address to map
/// paddr: The physical address to map
/// bits: An OR'd bitset containing the bits the leaf should have.
///       The bits should contain only the following:
///          Read, Write, Execute, User, and/or Global
///       The bits MUST include one or more of the following:
///          Read, Write, Execute
///       The valid bit automatically gets added.
/// level: 0 = 4KiB, 1 = 2MiB, 2 = 1GiB
pub fn map(
    root: &mut PageTable,
    vaddr: VirtualAddress,
    paddr: PhysicalAddress,
    bits: usize,
    level: usize,
) {
    assert!(bits & 0xe != 0);

    let mut v = &mut root.entries[vaddr.page_num(2)];
    for i in (level..2).rev() {
        if !v.is_valid() {
            let page = kalloc();
            v.set_entry((page as usize >> 2) | EntryBits::Valid.val());
        }
        let entry = ((v.get_entry() & !0x3ff) << 2) as *mut PageTableEntry;
        v = unsafe { entry.add(vaddr.page_num(i)).as_mut().unwrap() };
    }
    let entry = paddr.pg_entry() |
				bits |                    // Specified bits, such as User, Read, Write, etc
				EntryBits::Valid.val() |  // Valid bit
				EntryBits::Dirty.val() |  // Some machines require this to =1
				EntryBits::Access.val()   // Just like dirty, some machines require this
				;
    // Set the entry. V should be set to the correct pointer by the loop
    // above.
    v.set_entry(entry);
}

/// look up a virtual address in the page table an return the physical address
pub fn lookup(root: &PageTable, vaddr: VirtualAddress) -> Option<usize> {
    let mut v = &root.entries[vaddr.page_num(2)];

    for i in (0..=2).rev() {
        if v.is_invalid() {
            break;
        } else if v.is_leaf() {
            let off_mask = (1 << (12 + i * 9)) - 1;
            let vaddr_pgoff = vaddr.0 & off_mask;
            let addr = ((v.get_entry() << 2) as usize) & !off_mask;
            return Some(addr | vaddr_pgoff);
        }
        let entry = ((v.get_entry() & !0x3ff) << 2) as *const PageTableEntry;
        v = unsafe { entry.add(vaddr.page_num(i - 1)).as_ref().unwrap() };
    }

    None
}
