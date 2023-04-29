use crate::platform::PGSIZE;

/// a single PageTable with 512 entries
#[derive(Copy, Clone)]
#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [PageTableEntry; PGSIZE / 8],
}

static mut KERNEL_PAGETABLE: PageTable = PageTable::empty();

impl PageTable {
    pub fn as_addr(&self) -> usize {
        self.entries.as_ptr() as usize
    }

    pub const fn empty() -> PageTable {
        Self { entries: [PageTableEntry { entry: 0 }; PGSIZE / 8] }
    }

    pub fn len() -> usize {
        512
    }
}

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
    pub fn val(self) -> i64 {
        self as i64
    }
}

#[derive(Copy, Clone)]
pub struct PageTableEntry {
    pub entry: i64,
}
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

    pub fn set_entry(&mut self, entry: i64) {
        self.entry = entry;
    }

    pub fn get_entry(&self) -> i64 {
        self.entry
    }
}
