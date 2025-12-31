use aarch64_cpu::{
    asm::barrier::{self, dsb},
    registers::*,
};
use port::{
    fdt::DeviceTree,
    mem::{PhysAddr, PhysRange},
    pagealloc::PageAllocError,
};

use crate::{
    pre_mmu::util::putstr,
    vm::{
        AccessPermission, Entry, Mair, PageAllocator, PhysPage4K, RootPageTable, Shareable, Table,
    },
};

// These map to definitions in kernel.ld.
// In pre-MMU state, these will all map to physical addresses on aarch64.
unsafe extern "C" {
    static earlyvm_pagetables: [u64; 0];
    static eearlyvm_pagetables: [u64; 0];
}

fn earlyvm_pagetables_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(earlyvm_pagetables.as_ptr().addr() as u64) }
}

fn eearlyvm_pagetables_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(eearlyvm_pagetables.as_ptr().addr() as u64) }
}

fn earlyvm_pages_physrange() -> PhysRange {
    PhysRange::new(earlyvm_pagetables_physaddr(), eearlyvm_pagetables_physaddr())
}

#[unsafe(no_mangle)]
pub extern "C" fn init_vm(dtb_pa: u64) {
    // Parse the DTB before we set up memory so we can correctly map it
    let dt = unsafe { DeviceTree::from_usize(dtb_pa as usize).unwrap() };
    let _dtb_physrange = PhysRange::with_pa_len(PhysAddr::new(dtb_pa), dt.size());

    putstr("\nvm init: calling init_kernel_page_tables\n");

    let mut physpage_allocator = EarlyPageAllocator::new();

    // Manually set up page tables in rust instead of asm.  The next step in this
    // work will be to generate the tables dynamically, but for now we set up only
    // for raspberry pi.
    // *** This code is temporary ***

    // Constants for early uart setup
    const MMIO_BASE_RPI4: u64 = 0xfe000000;
    const GPIO: u64 = 0x00200000; // Offset from MMIO base

    let (kernelpt2_pa, kernelpt2) = if let Ok(page_pa) = physpage_allocator.alloc_physpage() {
        (page_pa, unsafe { &mut *(page_pa.0 as *mut Table) })
    } else {
        putstr("vm init: failed to alloc kernelpt2\n");
        panic!();
    };
    //.quad	(MMIO_BASE_RPI4)
    //+ (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_PXN|PT_MAIR_DEVICE)		// [496] (for mmio)
    kernelpt2.entries[496] = Entry::empty()
        .with_phys_addr(PhysAddr::new(MMIO_BASE_RPI4))
        .with_valid(true)
        .with_page_or_table(false)
        .with_accessed(true)
        .with_access_permission(AccessPermission::PrivRw)
        .with_shareable(Shareable::Inner)
        .with_uxn(true)
        .with_pxn(true)
        .with_mair_index(Mair::Device);
    //.quad	(MMIO_BASE_RPI4 + GPIO)
    // + (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_PXN|PT_MAIR_DEVICE)	// [497] (for mmio)
    kernelpt2.entries[497] = Entry::empty()
        .with_phys_addr(PhysAddr::new(MMIO_BASE_RPI4 + GPIO))
        .with_valid(true)
        .with_page_or_table(false)
        .with_accessed(true)
        .with_access_permission(AccessPermission::PrivRw)
        .with_shareable(Shareable::Inner)
        .with_uxn(true)
        .with_pxn(true)
        .with_mair_index(Mair::Device);
    //.quad	(kernelpt2)
    // + (PT_AF|PT_PAGE)	// [511] (recursive entry)
    kernelpt2.entries[511] =
        Entry::empty().with_phys_addr(kernelpt2_pa).with_valid(true).with_page_or_table(true);

    let (kernelpt3_pa, kernelpt3) = if let Ok(page_pa) = physpage_allocator.alloc_physpage() {
        (page_pa, unsafe { &mut *(page_pa.0 as *mut Table) })
    } else {
        putstr("vm init: failed to alloc kernelpt3\n");
        panic!();
    };
    //.quad	(0*2*GiB)
    // + (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_MAIR_NORMAL)	// [0] (for kernel)
    kernelpt3.entries[0] = Entry::empty()
        .with_phys_addr(PhysAddr::new(0))
        .with_valid(true)
        .with_page_or_table(false)
        .with_accessed(true)
        .with_access_permission(AccessPermission::PrivRw)
        .with_shareable(Shareable::Inner)
        .with_uxn(true)
        .with_mair_index(Mair::Normal);
    //.quad	(kernelpt2)
    // + (PT_AF|PT_PAGE)	// [3] (for mmio)
    kernelpt3.entries[3] =
        Entry::empty().with_phys_addr(kernelpt2_pa).with_valid(true).with_page_or_table(true);
    //.quad	(kernelpt3)
    // + (PT_AF|PT_PAGE)	// [511] (recursive entry)
    kernelpt3.entries[511] =
        Entry::empty().with_phys_addr(kernelpt3_pa).with_valid(true).with_page_or_table(true);

    let (kernelpt4_pa, kernelpt4) = if let Ok(page_pa) = physpage_allocator.alloc_physpage() {
        (page_pa, unsafe { &mut *(page_pa.0 as *mut RootPageTable) })
    } else {
        putstr("vm init: failed to alloc kernelpt4\n");
        panic!();
    };
    //.quad	(kernelpt3)
    // + (PT_AF|PT_PAGE)	// [256] (for kernel + mmio)
    kernelpt4.entries[256] =
        Entry::empty().with_phys_addr(kernelpt3_pa).with_valid(true).with_page_or_table(true);
    //.quad	(kernelpt4)
    // + (PT_AF|PT_PAGE)	// [511] (recursive entry)
    kernelpt4.entries[511] =
        Entry::empty().with_phys_addr(kernelpt4_pa).with_valid(true).with_page_or_table(true);

    // Early page tables for identity mapping the kernel physical addresses.
    // Once we've jumped to the higher half, this will no longer be used.

    let (physicalpt3_pa, physicalpt3) = if let Ok(page_pa) = physpage_allocator.alloc_physpage() {
        (page_pa, unsafe { &mut *(page_pa.0 as *mut Table) })
    } else {
        putstr("vm init: failed to alloc physicalpt3\n");
        panic!();
    };
    //.quad	(0*2*GiB)
    // + (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_MAIR_NORMAL)	// [0] (for kernel)
    physicalpt3.entries[0] = Entry::empty()
        .with_phys_addr(PhysAddr::new(0))
        .with_valid(true)
        .with_page_or_table(false)
        .with_accessed(true)
        .with_access_permission(AccessPermission::PrivRw)
        .with_shareable(Shareable::Inner)
        .with_uxn(true)
        .with_mair_index(Mair::Normal);

    let (physicalpt4_pa, physicalpt4) = if let Ok(page_pa) = physpage_allocator.alloc_physpage() {
        (page_pa, unsafe { &mut *(page_pa.0 as *mut RootPageTable) })
    } else {
        putstr("vm init: failed to alloc physicalpt4\n");
        panic!();
    };
    //.quad	(physicalpt3)
    // + (PT_AF|PT_PAGE)	// [0] (for kernel)
    physicalpt4.entries[0] =
        Entry::empty().with_phys_addr(physicalpt3_pa).with_valid(true).with_page_or_table(true);

    // AArch64 memory management examples
    //  https://developer.arm.com/documentation/102416/0100

    // AArch64 Address Translation
    //  https://developer.arm.com/documentation/100940/0101

    // The kernel has been loaded at the entrypoint, but the
    // addresses used in the elf are virtual addresses in the higher half.
    // If we try to access them, the CPU will trap, so the next step is to
    // enable the MMU and identity map the kernel virtual addresses to the
    // physical addresses that the kernel was loaded into.

    // The Aarch64 is super flexible.  We can have page tables (granules)
    // of 4, 16, or 64KiB.  If we assume 4KiB granules, we would have:
    //  [47-39] Index into L4 table, used to get address of the L3 table
    //  [38-30] Index into L3 table, used to get address of the L2 table
    //  [29-21] Index into L2 table, used to get address of the L1 table
    //  [20-12] Index into L1 table, used to get address of physical page
    //  [11-0]  Offset into physical page corresponding to virtual address
    // L4-L1 simply refers to the page table with L1 always being the last
    // to be translated, giving the address of the physical page.
    // With a 4KiB granule, each index is 9 bits, so there are 512 (2^9)
    // entries in each table.  In this example the physical page would
    // also be 4KiB.

    // If we reduce the number of page tables from 4 to 3 (L3 to L1),
    // we have 21 bits [20-0] for the physical page offset, giving 2MiB
    // pages.  If we reduce to 2 tables, we have 30 bits [29-0], giving
    // 1GiB pages.

    // If we use 16KiB granules, the virtual address is split as follows:
    //  [46-36] Index into L3 table, used to get address of the L2 table
    //  [35-25] Index into L2 table, used to get address of the L1 table
    //  [24-14] Index into L1 table, used to get address of physical page
    //  [13-0]  Offset into physical page corresponding to virtual address
    // The 14 bits in the offset results in 16KiB pages.  Each table is
    // 16KiB, consisting of 2048 entries, so requiring 11 bits per index.
    // If we instead use only 2 levels, that gives us bits [24-0] for the
    // offset into the physical page, which gives us 32MiB page size.

    // Finally, if we use 64KiB granules, the virtual address is split as
    // follows:
    //  [41-29] Index into L2 table, used to get address of the L1 table
    //  [28-16] Index into L1 table, used to get address of physical page
    //  [15-0]  Offset into physical page corresponding to virtual address
    // The 16 bits in the offset results in 64KiB pages.  Each table is
    // 64KiB, consisting of 8192 entries, so requiring 13 bits per index.
    // If we instead use only 1 level, that gives us bits [28-0] for the
    // offset into the physical page, which gives us 512MiB page size.

    // The address of the top level table is stored in the translation table
    // base registers.  ttbr0_el1 stores the address for the user space,
    // ttbr1_el1 stores the address for the kernel, both for EL1.
    // By default, ttbr1_el1 is used when the virtual address bit 55 is 1
    // otherwise ttbr0_el1 is used.

    // Memory attributes are set per page table entry, and are hierarchical,
    // so settings at a higher page affect those they reference.

    // Set up root tables for lower (ttbr0_el1) and higher (ttbr1_el1)
    // addresses.  kernelpt4 is the root of the page hierarchy for addresses
    // of the form 0xffff800000000000 (KZERO and above), while physicalpt4
    // handles 0x0000000000000000 until KZERO.  Although what we really
    // want is to move to virtual higher half addresses, we need to have
    // ttbr0_el1 identity mapped during the transition until the PC is also
    // in the higher half.  This is because the PC is still in the lower
    // half immediately after the MMU is enabled.  Once we enter rust-land,
    // we can define a new set of tables.

    putstr("vm init: switching\n");

    //enable_mmu(kernelpt4_pa, physicalpt4_pa);
    TTBR1_EL1.set(kernelpt4_pa.addr());
    TTBR0_EL1.set(physicalpt4_pa.addr());

    TCR_EL1.write(
        TCR_EL1::IPS::Bits_44
            + TCR_EL1::TG1::KiB_4
            + TCR_EL1::SH1::Inner
            + TCR_EL1::ORGN1::WriteBack_ReadAlloc_WriteAlloc_Cacheable
            + TCR_EL1::IRGN1::WriteBack_ReadAlloc_WriteAlloc_Cacheable
            + TCR_EL1::T1SZ.val(16)
            + TCR_EL1::TG0::KiB_4
            + TCR_EL1::SH0::Inner
            + TCR_EL1::ORGN0::WriteBack_ReadAlloc_WriteAlloc_Cacheable
            + TCR_EL1::IRGN0::WriteBack_ReadAlloc_WriteAlloc_Cacheable
            + TCR_EL1::T0SZ.val(16),
    );

    // Preset memory attributes.  This register stores 8 8-bit presets that are
    // referenced by index in the page table entries:
    //  [0] 0xff - Normal
    //  [1] 0x00 - Device (Non-gathering, non-reordering, no early write acknowledgement (most restrictive))
    MAIR_EL1.set(0x00ff);

    // https://forum.osdev.org/viewtopic.php?t=36412&p=303237
    #[cfg(not(test))]
    unsafe {
        // invalidate all TLB entries
        core::arch::asm!("tlbi vmalle1is")
    };

    dsb(barrier::ISH);

    putstr("vm init: complete\n");
}

struct EarlyPageAllocator {
    pages_pa: *mut PhysPage4K,
    num_pages: usize,
    next_page_idx: usize,
}

impl EarlyPageAllocator {
    fn new() -> Self {
        let earlyvm_pages_physrange = earlyvm_pages_physrange();
        let pages_start = earlyvm_pages_physrange.start().addr();
        let pages_pa = pages_start as *mut PhysPage4K;
        let num_pages = earlyvm_pages_physrange.size() / core::mem::size_of::<PhysPage4K>();
        for i in 0..num_pages {
            unsafe { (*pages_pa.add(i)).clear() };
        }

        Self { pages_pa, num_pages, next_page_idx: 0 }
    }
}

impl PageAllocator for EarlyPageAllocator {
    fn alloc_physpage(&mut self) -> Result<PhysAddr, PageAllocError> {
        if self.next_page_idx < self.num_pages {
            let next_page = unsafe { self.pages_pa.add(self.next_page_idx) };
            self.next_page_idx += 1;
            let pa = PhysAddr::new(next_page as u64);
            unsafe { &mut *(pa.0 as *mut PhysPage4K) }.clear();
            Ok(pa)
        } else {
            putstr("error:alloc_physpage:Out of space");
            Err(PageAllocError::OutOfSpace)
        }
    }
}
