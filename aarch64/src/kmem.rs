use port::mem::PhysAddr;

use crate::{param::KZERO, vm::Page4K};
use core::{mem, slice};

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

unsafe fn page_slice_mut<'a>(pstart: *mut Page4K, pend: *mut Page4K) -> &'a mut [Page4K] {
    let ustart = pstart.addr();
    let uend = pend.addr();
    const PAGE_SIZE: usize = mem::size_of::<Page4K>();
    assert_eq!(ustart % PAGE_SIZE, 0, "page_slice_mut: unaligned start page");
    assert_eq!(uend % PAGE_SIZE, 0, "page_slice_mut: unaligned end page");
    assert!(ustart < uend, "page_slice_mut: bad range");

    let len = (uend - ustart) / PAGE_SIZE;
    unsafe { slice::from_raw_parts_mut(ustart as *mut Page4K, len) }
}

pub fn early_pages() -> &'static mut [Page4K] {
    let early_start = early_pagetables_addr() as *mut Page4K;
    let early_end = eearly_pagetables_addr() as *mut Page4K;
    unsafe { page_slice_mut(early_start, early_end) }
}
