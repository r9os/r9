//! Initialization for ccNUMA node 0 and CPU 0 Mach
//!
//! Setup of the initial address space for node 0 and the Mach
//! for CPU 0 is tedious.  Plan 9 does this in assembly code,
//! but we prefer to do it in Rust.
//!
//! But in order to do it in Rust, we need a virtual address
//! space to execute from, as this is all 64-bit code that must
//! execute from long mode.  What to do?  Fortunately, early
//! boot forces the processor through a mode where we can call
//! rust code and create that address space.
//!
//! The assembly boot strap code needs to have access to an
//! identity mapping for the initial jump into long mode, and we
//! hardcode such a mapping covering the low 4GiB of the address
//! space, along with mapping the kernel image at its linked
//! addresses.  In this mode, kernel text is mapped at its
//! expected address range, but memory is still addressible at
//! physical addresses via the identity mapping, and we can
//! call this code to construct the address space for node 0 and
//! Mach 0, and remap the kernel, before we jump into `main`.
//!
//! The argument is a raw pointer to an array of "low memory"
//! page frames, which we understand cover the second megabyte
//! of RAM, and will provide the memory both for our early page
//! tables as well as the Mach and per-node data for node 0.
//!
//! One further complication remains: recall that our stack will
//! "live" in the Mach; where, then, is our stack, while running
//! this code?  Once fully booted, we will steal the page at
//! physical address 0x7000 to hold the startup code where any
//! additional CPUs will begin executing on startup.  Since we
//! are running well before any of that happens, we pre-use that
//! page as our initial stack for executing this code.
//!
//! The return value is the physical address of the newly
//! initialized PML4.

use crate::dat::{Gdt, HPA, Idt, Mach, PTable, Page, Tss};
use crate::trap;

const X: u64 = 0 << 63;
const NX: u64 = 1 << 63;
const L: u64 = 1 << 7;
const RO: u64 = 0 << 1;
const RW: u64 = 1 << 1;
const P: u64 = 1 << 0;

fn map_mach(pml1: &mut PTable, zp: HPA, idt: HPA, gdt: HPA, phys: [HPA; 29]) {
    const START: usize = 256;
    const LEN: usize = 64;
    let pml1 = &mut pml1.array_mut()[START..START + LEN];
    // Exception stacks and guard pages.
    for k in [0, 2, 4, 6].into_iter() {
        pml1[k] = NX | phys[k / 2].0 | RW | P;
        pml1[k + 1] = 0;
    }
    // Zero page.
    pml1[8] = NX | zp.0 | RO | P;
    // Empty space between the zero page and Mach stack
    for pte in &mut pml1[9..16].iter_mut() {
        *pte = 0;
    }
    // Mach stack
    for k in 16..32 {
        pml1[k] = NX | phys[k - 12].0 | RW | P;
    }
    // The actual in-use Mach data
    pml1[32] = NX | phys[20].0 | RW | P;

    // The page tables in the Mac
    for k in 33..40 {
        pml1[k] = NX | phys[k - 12].0 | RW | P;
    }
    // TSS is on a page by itself.
    pml1[40] = NX | phys[28].0 | RW | P;
    // The IDT.
    pml1[41] = NX | idt.0 | RO | P;
    // Empty space between the IDT and GDT.
    for pte in &mut pml1[42..48].iter_mut() {
        *pte = 0;
    }
    // The GDT.
    pml1[48] = NX | gdt.0 | RW | P;
    for pte in &mut pml1[49..] {
        *pte = NX | zp.0 | RO | P;
    }
}

fn ptr2hpa<T>(ptr: *const T) -> HPA {
    HPA(ptr.addr() as u64)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init0(lomem: *mut Page) -> HPA {
    let debug_stack = ptr2hpa(lomem.wrapping_add(0));
    let kpml3 = unsafe { &mut *lomem.add(1).cast::<PTable>() };
    let bp_stack = ptr2hpa(lomem.wrapping_add(2));
    let kpml2 = unsafe { &mut *lomem.add(3).cast::<PTable>() };
    let df_stack = ptr2hpa(lomem.wrapping_add(4));
    let kpml1 = unsafe { &mut *lomem.add(5).cast::<PTable>() };
    let nmi_stack = ptr2hpa(lomem.wrapping_add(6));
    let ncpml3 = unsafe { &mut *lomem.add(7).cast::<PTable>() }; // CPU/Node region
    let zero_page = unsafe { &mut *lomem.add(8).cast::<PTable>() };
    // Node region
    let npml2 = unsafe { &mut *lomem.add(9).cast::<PTable>() };
    let npml1 = unsafe { &mut *lomem.add(10).cast::<PTable>() };
    // CPU Region
    let cpml2 = unsafe { &mut *lomem.add(11).cast::<PTable>() };
    let cpml1 = unsafe { &mut *lomem.add(12).cast::<PTable>() };
    let arch_page = ptr2hpa(lomem.wrapping_add(13));
    let idt = unsafe { &mut *lomem.add(14).cast::<Idt>() };
    let gdt_page = unsafe { &mut *lomem.add(15) };
    // The kstack occupies lomem pages 16..32.
    // Following that are the page table structures that we
    // construct to map the kernel and point to the kernel
    // page tables in the recursive region.
    let mach_page = ptr2hpa(lomem.wrapping_add(32));
    let pml4 = unsafe { &mut *lomem.add(33).cast::<PTable>() };
    let pml3 = unsafe { &mut *lomem.add(34).cast::<PTable>() };
    let pml2 = unsafe { &mut *lomem.add(35).cast::<PTable>() };
    let pml1 = unsafe { &mut *lomem.add(36).cast::<PTable>() };
    let mpml3 = unsafe { &mut *lomem.add(37).cast::<PTable>() };
    let mpml2 = unsafe { &mut *lomem.add(38).cast::<PTable>() };
    let mpml1 = unsafe { &mut *lomem.add(39).cast::<PTable>() };

    const KCPUZERO: usize = 0xffff_ff00_0000_0000;
    const KMACH: usize = KCPUZERO + 0x0100_0000;
    const KMACHTSS: usize = KMACH + core::mem::offset_of!(Mach, tss);
    idt.init(trap::stubs());
    Gdt::init_in(gdt_page, KMACHTSS as *mut Tss);
    let mut phys = [HPA::from_phys(0); 29];
    phys[0] = debug_stack;
    phys[1] = bp_stack;
    phys[2] = df_stack;
    phys[3] = nmi_stack;
    for k in 16..32 {
        phys[k - 12] = ptr2hpa(lomem.wrapping_add(k));
    }
    phys[20] = mach_page;
    phys[21] = ptr2hpa(pml4);
    phys[22] = ptr2hpa(pml3);
    phys[23] = ptr2hpa(pml2);
    phys[24] = ptr2hpa(pml1);
    phys[25] = ptr2hpa(mpml3);
    phys[26] = ptr2hpa(mpml2);
    phys[27] = ptr2hpa(mpml1);
    phys[28] = arch_page;
    map_mach(cpml1, ptr2hpa(zero_page), ptr2hpa(idt), ptr2hpa(gdt_page), phys);

    // These assignments set up the recursive mapping region so
    // that the PML4 is itself mapped.
    pml4.array_mut()[508] = NX | ptr2hpa(pml3).0 | RW | P;
    pml3.array_mut()[508] = NX | ptr2hpa(pml2).0 | RW | P;
    pml2.array_mut()[508] = NX | ptr2hpa(pml1).0 | RW | P;
    pml1.array_mut()[508] = NX | ptr2hpa(pml4).0 | RW | P;

    // Map the kernel.
    pml4.array_mut()[511] = X | ptr2hpa(kpml3).0 | RW | P;
    kpml3.array_mut()[510] = X | ptr2hpa(kpml2).0 | RW | P;
    kpml2.array_mut()[0] = NX | ptr2hpa(kpml1).0 | RW | P;
    kpml2.array_mut()[1] = X | 0x0020_0000 | L | RO | P;
    kpml2.array_mut()[2] = NX | 0x0040_0000 | L | RO | P;
    kpml2.array_mut()[3] = NX | 0x0060_0000 | L | RW | P;
    kpml1.array_mut()[0] = NX | 0x0000_0000 | RW | P;
    kpml1.array_mut()[7] = NX | 0x0007_0000 | RW | P;
    for k in 32..64 {
        kpml1.array_mut()[k] = NX | (k as u64 * 4096) | RW | P;
    }

    // Map the CPU/node area
    pml4.array_mut()[510] = NX | ptr2hpa(ncpml3).0 | RW | P;
    ncpml3.array_mut()[0] = NX | ptr2hpa(cpml2).0 | RW | P;
    cpml2.array_mut()[0] = NX | ptr2hpa(cpml1).0 | RW | P;
    ncpml3.array_mut()[256] = NX | ptr2hpa(npml2).0 | RW | P;
    npml2.array_mut()[0] = NX | ptr2hpa(npml1).0 | RW | P;

    // These assignments set up the empty page tables that
    // cover the "Mapping Region", which is what Hypatia calls
    // the "Linkage Segment").
    pml4.array_mut()[509] = NX | ptr2hpa(mpml3).0 | RW | P;
    mpml3.array_mut()[511] = NX | ptr2hpa(mpml2).0 | RW | P;
    mpml2.array_mut()[511] = NX | ptr2hpa(mpml1).0 | RW | P;

    // Set up the recursive region.
    //
    // Within the recurisve region:
    //  1. The right-most PML1 maps the PML4 and non-rec PML3s
    //  2. The right-most PML2 roots rec PML3 and non-rec PML2s
    //  3. The right-most PML3 roots rec PML2 and non-rec PML1s
    // At each level, the index for each sub-tree is the PML3
    // index.  Within those sublevels, indices are for the next
    // higher actual map address.
    pml1.array_mut()[511] = NX | ptr2hpa(kpml3).0 | RW | P;
    pml1.array_mut()[510] = NX | ptr2hpa(ncpml3).0 | RW | P;
    pml1.array_mut()[509] = NX | ptr2hpa(mpml3).0 | RW | P;

    // Map the mapping region tables into the recursive region.
    let mpml1_rpml2 = unsafe { &mut *lomem.add(40).cast::<PTable>() };
    let mpml1_rpml1 = unsafe { &mut *lomem.add(41).cast::<PTable>() };
    let mpml2_rpml1 = unsafe { &mut *lomem.add(42).cast::<PTable>() };

    pml3.array_mut()[509] = NX | ptr2hpa(mpml1_rpml2).0 | RW | P;
    mpml1_rpml2.array_mut()[511] = NX | ptr2hpa(mpml1_rpml1).0 | RW | P;
    mpml1_rpml1.array_mut()[511] = NX | ptr2hpa(mpml1).0 | RW | P;
    pml2.array_mut()[509] = NX | ptr2hpa(mpml2_rpml1).0 | RW | P;
    mpml2_rpml1.array_mut()[511] = NX | ptr2hpa(mpml2).0 | RW | P;

    // Map the for CPU/node region tables.
    let ncpml2_rpml1 = unsafe { &mut *lomem.add(43).cast::<PTable>() };
    let ncpml1_rpml2 = unsafe { &mut *lomem.add(44).cast::<PTable>() };
    let cpml1_rpml1 = unsafe { &mut *lomem.add(45).cast::<PTable>() };
    let npml1_rpml1 = unsafe { &mut *lomem.add(46).cast::<PTable>() };

    pml3.array_mut()[510] = NX | ptr2hpa(ncpml1_rpml2).0 | RW | P;
    ncpml1_rpml2.array_mut()[0] = NX | ptr2hpa(cpml1_rpml1).0 | RW | P;
    cpml1_rpml1.array_mut()[0] = NX | ptr2hpa(cpml1).0 | RW | P;
    ncpml1_rpml2.array_mut()[256] = NX | ptr2hpa(npml1_rpml1).0 | RW | P;
    npml1_rpml1.array_mut()[0] = NX | ptr2hpa(npml1).0 | RW | P;

    pml2.array_mut()[510] = NX | ptr2hpa(ncpml2_rpml1).0 | RW | P;
    ncpml2_rpml1.array_mut()[0] = NX | ptr2hpa(cpml2).0 | RW | P;
    ncpml2_rpml1.array_mut()[256] = NX | ptr2hpa(npml2).0 | RW | P;

    // Map the kernel tables.
    let kpml2_rpml1 = unsafe { &mut *lomem.add(47).cast::<PTable>() };
    let kpml1_rpml2 = unsafe { &mut *lomem.add(48).cast::<PTable>() };
    let kpml1_rpml1 = unsafe { &mut *lomem.add(49).cast::<PTable>() };

    pml3.array_mut()[511] = NX | ptr2hpa(kpml1_rpml2).0 | RW | P;
    kpml1_rpml2.array_mut()[510] = NX | ptr2hpa(kpml1_rpml1).0 | RW | P;
    kpml1_rpml1.array_mut()[0] = NX | ptr2hpa(kpml1).0 | RW | P;
    pml2.array_mut()[511] = NX | ptr2hpa(kpml2_rpml1).0 | RW | P;
    kpml2_rpml1.array_mut()[0] = NX | ptr2hpa(kpml2).0 | RW | P;

    ptr2hpa(pml4)
}
