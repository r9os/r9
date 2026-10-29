//! aarch64's implementation of the traits port::proc needs.

use crate::kmap::kmap;
use crate::kstack::{self, KStack};
use crate::mach::mach;
use crate::param;
use crate::swtch::{self, Label};
use crate::trap::{self, TrapFrame};
use crate::vm::{
    self, AddressSpace, Entry, PageSize, PageTableError, PhysPageAllocator, RootPageTableType,
    VaMapping, VmTraitImpl,
};
use port::dat::RunQueue;
use port::dat::{Access, MMU, Perm, Proc, ProcArch, PutError};
use port::exec::UserRegs;
use port::mcslock::Lock;
use port::mem::{PAGE_SIZE_4K, PhysAddr, PhysRange};
use port::pagealloc::PageAllocError;

pub struct Aarch64;

/// aarch64's current process, for port's code that isn't generic over it.
static CURRENT: port::proc::Current = port::proc::Current::of::<Aarch64>();

/// Register aarch64's current process with port.
pub fn init() {
    port::proc::init_current(&CURRENT);
}

/// A process's FP/SIMD registers: q0-q31, then FPCR and FPSR, laid out as
/// fp_save and fp_restore address them.  A new process's are zero.
#[derive(Default)]
#[repr(C, align(16))]
pub struct Fp {
    q: [u128; 32],
    fpcr: u64,
    fpsr: u64,
}

const _: () = assert!(core::mem::offset_of!(Fp, fpcr) == 512);
const _: () = assert!(core::mem::offset_of!(Fp, fpsr) == 520);

impl MMU for Aarch64 {
    type Pmmu = AddressSpace;

    const UADDRTOP: usize = param::UADDRTOP;
    const UTZERO: usize = param::UTZERO;
    const USTKTOP: usize = param::USTKTOP;
    const USTKSIZE: usize = param::USTKSIZE;

    fn newpmmu() -> Result<AddressSpace, PageAllocError> {
        AddressSpace::new()
    }

    fn user_page_ok(va: usize, access: Access) -> bool {
        vm::user_page_ok(va, access)
    }

    fn put(va: usize, pa: PhysAddr, perm: Perm) -> Result<(), PutError> {
        if perm == Perm::Text {
            let k = kmap(pa);
            vm::sync_icache(k.va(), PAGE_SIZE_4K);
        }
        let entry = match perm {
            Perm::Text => Entry::ro_user_text(),
            Perm::Rodata => Entry::ro_user_data(),
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

impl ProcArch for Aarch64 {
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

    fn up() -> *mut Proc<Aarch64> {
        // SAFETY: mach is this CPU's, and only this CPU uses it.
        unsafe { (*mach()).up }
    }

    fn set_up(p: *mut Proc<Aarch64>) {
        // SAFETY: as above.
        unsafe { (*mach()).up = p };
    }

    fn sched() -> *mut Label {
        // SAFETY: as above.
        unsafe { &raw mut (*mach()).sched }
    }

    type Fp = Fp;

    #[cfg_attr(test, expect(unused_variables, reason = "host tests have no FP state"))]
    fn fp_save(fp: &mut Fp) {
        #[cfg(not(test))]
        // SAFETY: fp is 528 bytes, 16-byte aligned, and writable, and storing
        // the registers changes none of them.
        unsafe {
            core::arch::asm!(
                ".arch_extension fp",
                "stp q0, q1, [{fp}, #0]",
                "stp q2, q3, [{fp}, #32]",
                "stp q4, q5, [{fp}, #64]",
                "stp q6, q7, [{fp}, #96]",
                "stp q8, q9, [{fp}, #128]",
                "stp q10, q11, [{fp}, #160]",
                "stp q12, q13, [{fp}, #192]",
                "stp q14, q15, [{fp}, #224]",
                "stp q16, q17, [{fp}, #256]",
                "stp q18, q19, [{fp}, #288]",
                "stp q20, q21, [{fp}, #320]",
                "stp q22, q23, [{fp}, #352]",
                "stp q24, q25, [{fp}, #384]",
                "stp q26, q27, [{fp}, #416]",
                "stp q28, q29, [{fp}, #448]",
                "stp q30, q31, [{fp}, #480]",
                "mrs {t}, fpcr",
                "str {t}, [{fp}, #512]",
                "mrs {t}, fpsr",
                "str {t}, [{fp}, #520]",
                fp = in(reg) fp,
                t = out(reg) _,
                options(nostack),
            );
        }
    }

    #[cfg_attr(test, expect(unused_variables, reason = "host tests have no FP state"))]
    fn fp_restore(fp: &Fp) {
        #[cfg(not(test))]
        // SAFETY: fp is 528 bytes, 16-byte aligned, and readable.  The asm
        // overwrites v0-v31 without declaring them, which is sound only
        // because the target has no neon: the compiler can't allocate them,
        // so nothing it generates holds a value there.
        unsafe {
            core::arch::asm!(
                ".arch_extension fp",
                "ldp q0, q1, [{fp}, #0]",
                "ldp q2, q3, [{fp}, #32]",
                "ldp q4, q5, [{fp}, #64]",
                "ldp q6, q7, [{fp}, #96]",
                "ldp q8, q9, [{fp}, #128]",
                "ldp q10, q11, [{fp}, #160]",
                "ldp q12, q13, [{fp}, #192]",
                "ldp q14, q15, [{fp}, #224]",
                "ldp q16, q17, [{fp}, #256]",
                "ldp q18, q19, [{fp}, #288]",
                "ldp q20, q21, [{fp}, #320]",
                "ldp q22, q23, [{fp}, #352]",
                "ldp q24, q25, [{fp}, #384]",
                "ldp q26, q27, [{fp}, #416]",
                "ldp q28, q29, [{fp}, #448]",
                "ldp q30, q31, [{fp}, #480]",
                "ldr {t}, [{fp}, #512]",
                "msr fpcr, {t}",
                "ldr {t}, [{fp}, #520]",
                "msr fpsr, {t}",
                fp = in(reg) fp,
                t = out(reg) _,
                options(nostack, readonly),
            );
        }
    }

    /// wfi ends on a pending IRQ even with IRQs masked: an IRQ is a wakeup
    /// event regardless of PSTATE.I (Arm ARM DDI 0487 M.d, D1.7.2.1).
    fn idle() {
        aarch64_cpu::asm::wfi();
    }

    fn run_queue() -> &'static Lock<RunQueue<Aarch64>> {
        static RUN_QUEUE: Lock<RunQueue<Aarch64>> = Lock::new("run_queue", RunQueue::new());
        &RUN_QUEUE
    }
}

/// Enter the current process with `regs`.  Call it only from a process's start
/// function, after its text and stack are mapped.
pub fn enter_user(regs: UserRegs) -> ! {
    // SAFETY: up is this process, which is running, and its context started
    // below the space to_user uses at the top of its stack.
    let kstack_top = unsafe { (*Aarch64::up()).kstack().top() };
    // SAFETY: as above, and the caller mapped the text and stack.
    unsafe { trap::to_user(kstack_top, &regs) }
}
