use crate::param::KZERO;
use port::mem::{PhysAddr, PhysRange};

// These map to definitions in kernel.ld
unsafe extern "C" {
    static eboottext_pa: [u64; 0];
    static text_pa: [u64; 0];
    static etext_pa: [u64; 0];
    static rodata_pa: [u64; 0];
    static erodata_pa: [u64; 0];
    static data_pa: [u64; 0];
    static edata_pa: [u64; 0];
    static bss_pa: [u64; 0];
    static ebss_pa: [u64; 0];
}

fn base_physaddr() -> PhysAddr {
    PhysAddr::new(0)
}

fn eboottext_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(eboottext_pa.as_ptr().addr() as u64) }
}

fn text_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(text_pa.as_ptr().addr() as u64) }
}

fn etext_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(etext_pa.as_ptr().addr() as u64) }
}

fn rodata_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(rodata_pa.as_ptr().addr() as u64) }
}

fn erodata_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(erodata_pa.as_ptr().addr() as u64) }
}

fn data_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(data_pa.as_ptr().addr() as u64) }
}

fn edata_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(edata_pa.as_ptr().addr() as u64) }
}

fn bss_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(bss_pa.as_ptr().addr() as u64) }
}

fn ebss_physaddr() -> PhysAddr {
    unsafe { PhysAddr::new(ebss_pa.as_ptr().addr() as u64) }
}

pub fn boottext_physrange() -> PhysRange {
    PhysRange::new(base_physaddr(), eboottext_physaddr())
}

pub fn text_physrange() -> PhysRange {
    PhysRange::new(text_physaddr(), etext_physaddr())
}

pub fn rodata_physrange() -> PhysRange {
    PhysRange::new(rodata_physaddr(), erodata_physaddr())
}

pub fn data_physrange() -> PhysRange {
    PhysRange::new(data_physaddr(), edata_physaddr())
}

pub fn bss_physrange() -> PhysRange {
    PhysRange::new(bss_physaddr(), ebss_physaddr())
}

pub fn total_kernel_physrange() -> PhysRange {
    PhysRange::new(base_physaddr(), ebss_physaddr())
}

/// Return the physical address of `a`, which must be mapped at KZERO + its
/// physical address.  Only the kernel image, the DTB and pages the kernel maps
/// for itself are mapped there.  Other pages are reached through kmap, or, for
/// page tables, through the recursive window.
pub fn paddr<T>(a: *const T) -> PhysAddr {
    let va = a.addr();
    debug_assert!(va >= KZERO, "paddr: va {va:#x} must be >= KZERO ({KZERO:#x})");
    PhysAddr::new((va - KZERO) as u64)
}
