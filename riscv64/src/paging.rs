use crate::memory::kalloc;

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
pub fn map(root: &mut PageTable, vaddr: usize, paddr: usize, bits: usize, level: usize) {
    // Make sure that Read, Write, or Execute have been provided
    // otherwise, we'll leak memory and always create a page fault.
    assert!(bits & 0xe != 0);
    // Extract out each VPN from the virtual address
    // On the virtual address, each VPN is exactly 9 bits,
    // which is why we use the mask 0x1ff = 0b1_1111_1111 (9 bits)
    let vpn = [
        // VPN[0] = vaddr[20:12]
        (vaddr >> 12) & 0x1ff,
        // VPN[1] = vaddr[29:21]
        (vaddr >> 21) & 0x1ff,
        // VPN[2] = vaddr[38:30]
        (vaddr >> 30) & 0x1ff,
    ];

    // Just like the virtual address, extract the physical address
    // numbers (PPN). However, PPN[2] is different in that it stores
    // 26 bits instead of 9. Therefore, we use,
    // 0x3ff_ffff = 0b11_1111_1111_1111_1111_1111_1111 (26 bits).
    let ppn = [
        // PPN[0] = paddr[20:12]
        (paddr >> 12) & 0x1ff,
        // PPN[1] = paddr[29:21]
        (paddr >> 21) & 0x1ff,
        // PPN[2] = paddr[55:30]
        (paddr >> 30) & 0x3ff_ffff,
    ];
    // We will use this as a floating reference so that we can set
    // individual entries as we walk the table.
    let mut v = &mut root.entries[vpn[2]];
    // Now, we're going to traverse the page table and set the bits
    // properly. We expect the root to be valid, however we're required to
    // create anything beyond the root.
    // In Rust, we create a range iterator using the .. operator.
    // The .rev() will reverse the iteration since we need to start with
    // VPN[2] The .. operator is inclusive on start but exclusive on end.
    // So, (0..2) will iterate 0 and 1.
    for i in (level..2).rev() {
        if !v.is_valid() {
            // Allocate a page
            let page = kalloc();
            // The page is already aligned by 4,096, so store it
            // directly The page is stored in the entry shifted
            // right by 2 places.
            v.set_entry((page as usize >> 2) | EntryBits::Valid.val());
        }
        let entry = ((v.get_entry() & !0x3ff) << 2) as *mut PageTableEntry;
        v = unsafe { entry.add(vpn[i]).as_mut().unwrap() };
    }
    // When we get here, we should be at VPN[0] and v should be pointing to
    // our entry.
    // The entry structure is Figure 4.18 in the RISC-V Privileged
    // Specification
    let entry = (ppn[2] << 28) |   // PPN[2] = [53:28]
	            (ppn[1] << 19) |   // PPN[1] = [27:19]
				(ppn[0] << 10) |   // PPN[0] = [18:10]
				bits |                    // Specified bits, such as User, Read, Write, etc
				EntryBits::Valid.val() |  // Valid bit
				EntryBits::Dirty.val() |  // Some machines require this to =1
				EntryBits::Access.val()   // Just like dirty, some machines require this
				;
    // Set the entry. V should be set to the correct pointer by the loop
    // above.
    v.set_entry(entry);
}

pub fn virt_to_phys(root: &PageTable, vaddr: usize) -> Option<usize> {
    let vpn = [(vaddr >> 12) & 0x1ff, (vaddr >> 21) & 0x1ff, (vaddr >> 30) & 0x1ff];

    let mut v = &root.entries[vpn[2]];

    for i in (0..=2).rev() {
        if v.is_invalid() {
            break;
        } else if v.is_leaf() {
            let off_mask = (1 << (12 + i * 9)) - 1;
            let vaddr_pgoff = vaddr & off_mask;
            let addr = ((v.get_entry() << 2) as usize) & !off_mask;
            return Some(addr | vaddr_pgoff);
        }
        let entry = ((v.get_entry() & !0x3ff) << 2) as *const PageTableEntry;
        v = unsafe { entry.add(vpn[i - 1]).as_ref().unwrap() };
    }

    None
}
