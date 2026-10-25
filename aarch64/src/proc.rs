//! aarch64's implementation of the traits port::proc needs.

use crate::kmap::kmap;
use crate::kstack::{self, KStack};
use crate::mach::mach;
use crate::param;
use crate::swtch::{self, Label};
use crate::trap::TrapFrame;
use crate::vm::{
    self, AddressSpace, Entry, PageSize, PageTableError, PhysPageAllocator, RootPageTableType,
    VaMapping, VmTraitImpl,
};
use port::dat::{Access, MMU, Perm, Proc, ProcArch, PutError};
use port::mem::{PAGE_SIZE_4K, PhysAddr, PhysRange};
use port::pagealloc::PageAllocError;

pub struct Arm64;

impl MMU for Arm64 {
    type Pmmu = AddressSpace;

    const UADDRTOP: usize = param::UADDRTOP;
    const USTKTOP: usize = param::USTKTOP;
    const USTKSIZE: usize = param::USTKSIZE;

    fn newpmmu() -> Result<AddressSpace, PageAllocError> {
        AddressSpace::new()
    }

    fn user_page_ok(va: usize, access: Access) -> bool {
        vm::user_page_ok(va, access)
    }

    fn put(va: usize, pa: PhysAddr, perm: Perm) -> Result<(), PutError> {
        let entry = match perm {
            Perm::Text => Entry::ro_user_text(),
            Perm::Data => Entry::rw_user_data(),
        };
        let mapped = vm::map_phys_range(
            &mut PhysPageAllocator {},
            &mut VmTraitImpl {},
            "put",
            &PhysRange::with_pa_len(pa, PAGE_SIZE_4K),
            VaMapping::Addr(va),
            entry,
            PageSize::Page4K,
            RootPageTableType::User,
        );
        match mapped {
            Ok(_) => Ok(()),
            Err(PageTableError::AlreadyMapped) => Err(PutError::AlreadyMapped),
            Err(PageTableError::AllocationFailed(_)) => Err(PutError::NoMemory),
            Err(err) => panic!("MMU::put: couldn't map {va:#x}: {err:?}"),
        }
    }

    fn unput(va: usize) -> Option<PhysAddr> {
        vm::unmap_user_page(va)
    }

    fn kmap<R>(pa: PhysAddr, f: impl FnOnce(*mut u8) -> R) -> R {
        let k = kmap(pa);
        f(k.va() as *mut u8)
    }

    fn flush() {
        unimplemented!("MMU::flush")
    }

    fn flushva(_ptr: *const ()) {
        unimplemented!("MMU::flushva")
    }

    unsafe fn switch(pmmu: &AddressSpace) {
        // SAFETY: the caller's guarantee.
        unsafe { vm::switch(pmmu) };
    }

    unsafe fn release(pmmu: AddressSpace, free_leaf: impl FnMut(PhysAddr)) {
        // SAFETY: the caller's guarantee.
        unsafe { vm::release(pmmu, free_leaf) }.expect("couldn't release an address space");
    }
}

impl ProcArch for Arm64 {
    type Label = Label;
    type KStack = KStack;

    fn kstack_alloc() -> Result<KStack, PageAllocError> {
        kstack::alloc()
    }

    unsafe fn kstack_free(stack: KStack) {
        // SAFETY: the caller's guarantee.
        unsafe { kstack::free(stack) };
    }

    /// The context starts below a TrapFrame's worth of space at the top of
    /// the stack, where to_user builds the frame it enters user mode with, as
    /// Plan 9 keeps the top of each kernel stack for its Ureg.
    fn new_context(entry: extern "C" fn() -> !, stack: &KStack) -> Label {
        Label::new_context(entry, stack.top() - size_of::<TrapFrame>())
    }

    unsafe fn swtch(save: *mut Label, next: *const Label) {
        // SAFETY: the caller's guarantee.
        unsafe { swtch::swtch(save, next) };
    }

    fn up() -> *mut Proc<Arm64> {
        // SAFETY: mach is this CPU's, and only this CPU uses it.
        unsafe { (*mach()).up }
    }

    fn set_up(p: *mut Proc<Arm64>) {
        // SAFETY: as above.
        unsafe { (*mach()).up = p };
    }

    fn sched() -> *mut Label {
        // SAFETY: as above.
        unsafe { &raw mut (*mach()).sched }
    }
}
