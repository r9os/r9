use crate::param::KZERO;
use port::mem::{PhysAddr, PhysRange};

// These map to definitions in kernel.ld
extern "C" {
    static etext: [u64; 0];
    static erodata: [u64; 0];
    static ebss: [u64; 0];
    static early_pagetables: [u64; 0];
    static eearly_pagetables: [u64; 0];
}

pub fn text_addr() -> usize {
    0xffff_8000_0000_0000
}

pub fn etext_addr() -> usize {
    unsafe { etext.as_ptr().addr() }
}

pub fn erodata_addr() -> usize {
    unsafe { erodata.as_ptr().addr() }
}

pub fn ebss_addr() -> usize {
    unsafe { ebss.as_ptr().addr() }
}

pub fn early_pagetables_addr() -> usize {
    unsafe { early_pagetables.as_ptr().addr() }
}

pub fn eearly_pagetables_addr() -> usize {
    unsafe { eearly_pagetables.as_ptr().addr() }
}

pub const fn physaddr_as_virt(pa: PhysAddr) -> usize {
    (pa.addr() as usize).wrapping_add(KZERO)
}

pub const fn physaddr_as_ptr_mut<T>(pa: PhysAddr) -> *mut T {
    physaddr_as_virt(pa) as *mut T
}

pub const fn from_virt_to_physaddr(va: usize) -> PhysAddr {
    PhysAddr::new((va - KZERO) as u64)
}

pub fn from_ptr_to_physaddr<T>(a: *const T) -> PhysAddr {
    from_virt_to_physaddr(a.addr())
}

pub fn early_pages_range() -> PhysRange {
    PhysRange::new(
        from_virt_to_physaddr(early_pagetables_addr()),
        from_virt_to_physaddr(eearly_pagetables_addr()),
    )
}
