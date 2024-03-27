use crate::param::KZERO;
use port::mem::{PhysAddr, PhysRange, VirtRange};

// These map to definitions in kernel.ld
extern "C" {
    static etext: [u64; 0];
    static erodata: [u64; 0];
    static ebss: [u64; 0];
    static early_pagetables: [u64; 0];
    static eearly_pagetables: [u64; 0];
    static heap: [u64; 0];
    static eheap: [u64; 0];
}

fn text_addr() -> usize {
    0xffff_8000_0000_0000
}

fn etext_addr() -> usize {
    unsafe { etext.as_ptr().addr() }
}

fn erodata_addr() -> usize {
    unsafe { erodata.as_ptr().addr() }
}

fn ebss_addr() -> usize {
    unsafe { ebss.as_ptr().addr() }
}

fn early_pagetables_addr() -> usize {
    unsafe { early_pagetables.as_ptr().addr() }
}

fn eearly_pagetables_addr() -> usize {
    unsafe { eearly_pagetables.as_ptr().addr() }
}

fn heap_addr() -> usize {
    unsafe { heap.as_ptr().addr() }
}

fn eheap_addr() -> usize {
    unsafe { eheap.as_ptr().addr() }
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

pub fn kernel_text_physrange() -> PhysRange {
    PhysRange(from_virt_to_physaddr(text_addr())..from_virt_to_physaddr(etext_addr()))
}

pub fn kernel_data_physrange() -> PhysRange {
    PhysRange::with_len(from_virt_to_physaddr(etext_addr()).addr(), erodata_addr() - etext_addr())
}

pub fn kernel_bss_physrange() -> PhysRange {
    PhysRange::with_len(from_virt_to_physaddr(erodata_addr()).addr(), ebss_addr() - erodata_addr())
}

pub fn kernel_heap_physrange() -> PhysRange {
    PhysRange::with_len(from_virt_to_physaddr(heap_addr()).addr(), eheap_addr() - heap_addr())
}

pub fn kernel_heap_virtrange() -> VirtRange {
    VirtRange::with_len(heap_addr(), eheap_addr() - heap_addr())
}

pub fn early_pages_physrange() -> PhysRange {
    PhysRange::new(
        from_virt_to_physaddr(early_pagetables_addr()),
        from_virt_to_physaddr(eearly_pagetables_addr()),
    )
}
