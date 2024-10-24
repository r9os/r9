use crate::param::KZERO;
use port::mem::{PhysAddr, PhysRange};

// These map to definitions in kernel.ld
extern "C" {
    static eboottext: [u64; 0];
    static text: [u64; 0];
    static etext: [u64; 0];
    static rodata: [u64; 0];
    static erodata: [u64; 0];
    static data: [u64; 0];
    static edata: [u64; 0];
    static bss: [u64; 0];
    static ebss: [u64; 0];
    static end: [u64; 0];
    static early_pagetables: [u64; 0];
    static eearly_pagetables: [u64; 0];
}

fn base_addr() -> usize {
    0xffff_8000_0000_0000
}

fn eboottext_addr() -> usize {
    unsafe { eboottext.as_ptr().addr() }
}

fn text_addr() -> usize {
    unsafe { text.as_ptr().addr() }
}

fn etext_addr() -> usize {
    unsafe { etext.as_ptr().addr() }
}

fn rodata_addr() -> usize {
    unsafe { rodata.as_ptr().addr() }
}

fn erodata_addr() -> usize {
    unsafe { erodata.as_ptr().addr() }
}

fn data_addr() -> usize {
    unsafe { data.as_ptr().addr() }
}

fn edata_addr() -> usize {
    unsafe { edata.as_ptr().addr() }
}

fn bss_addr() -> usize {
    unsafe { bss.as_ptr().addr() }
}

fn ebss_addr() -> usize {
    unsafe { ebss.as_ptr().addr() }
}

fn end_addr() -> usize {
    unsafe { end.as_ptr().addr() }
}

fn early_pagetables_addr() -> usize {
    unsafe { early_pagetables.as_ptr().addr() }
}

fn eearly_pagetables_addr() -> usize {
    unsafe { eearly_pagetables.as_ptr().addr() }
}

pub fn boottext_range() -> PhysRange {
    PhysRange(from_virt_to_physaddr(base_addr())..from_virt_to_physaddr(eboottext_addr()))
}

pub fn text_range() -> PhysRange {
    PhysRange(from_virt_to_physaddr(text_addr())..from_virt_to_physaddr(etext_addr()))
}

pub fn rodata_range() -> PhysRange {
    PhysRange(from_virt_to_physaddr(rodata_addr())..from_virt_to_physaddr(erodata_addr()))
}

pub fn data_range() -> PhysRange {
    PhysRange(from_virt_to_physaddr(data_addr())..from_virt_to_physaddr(edata_addr()))
}

pub fn bss_range() -> PhysRange {
    PhysRange(from_virt_to_physaddr(bss_addr())..from_virt_to_physaddr(ebss_addr()))
}

pub fn total_kernel_range() -> PhysRange {
    PhysRange(from_virt_to_physaddr(base_addr())..from_virt_to_physaddr(end_addr()))
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
