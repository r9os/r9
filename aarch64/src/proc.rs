//! aarch64's implementation of the traits port::proc needs.

use crate::kstack::{self, KStack};
use crate::mach::mach;
use crate::swtch::{self, Label};
use crate::trap::TrapFrame;
use crate::vm::{self, AddressSpace};
use port::dat::{MMU, Proc, ProcArch};
use port::mem::PhysAddr;
use port::pagealloc::PageAllocError;

pub struct Arm64;

impl MMU for Arm64 {
    type Pmmu = AddressSpace;

    fn newpmmu() -> Result<AddressSpace, PageAllocError> {
        AddressSpace::new()
    }

    fn put() {
        unimplemented!("MMU::put")
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
