//! "Vestigial Segmented Virtual Memory"
//!
//! This module is a bit unfortunate; it's simply a grab-bag of
//! x86 legacy.  Nominally, all of this ought to go into dat.rs,
//! but that's already busy enough without polluting it with
//! this goo.

use crate::cpu;
use crate::dat::{Mach, MachMode, Page, Stack};
use crate::trap;
use crate::trap::BREAKPOINT_TRAPNO;
use crate::trap::{DEBUG_TRAPNO, DOUBLE_FAULT_TRAPNO, NMI_TRAPNO};

use bit_field::BitField;
use zerocopy::FromZeros;

pub mod seg {
    use super::{Gdt, IstIndex, MachMode, Tss, trap};

    use bit_field::BitField;
    use bitstruct::bitstruct;
    use zerocopy::FromZeros;

    bitstruct! {
        /// Segment Descriptors describe memory segments in the GDT.
        #[derive(Clone, Copy, Debug, FromZeros)]
        #[repr(transparent)]
        pub struct Descr(u64) {
            reserved0: u32 = 0..32;
            reserved1: u8 = 32..40;
            pub accessed: bool = 40;
            pub readable: bool = 41;
            pub conforming: bool = 42;
            code: bool = 43;
            system: bool = 44;
            iopl: MachMode = 45..47;
            pub present: bool = 47;
            reserved2: u8 = 48..52;
            available: bool = 52;
            long: bool = 53;
            default32: bool = 54;
            granularity: bool = 55;
            reserved3: u8 = 56..64;
        }
    }

    impl Descr {
        pub const fn empty() -> Descr {
            Descr(0)
        }
        pub const fn null() -> Descr {
            Self::empty()
        }
        pub fn ktext64() -> Descr {
            Self::empty()
                .with_system(true)
                .with_code(true)
                .with_readable(true)
                .with_present(true)
                .with_conforming(true)
                .with_long(true)
                .with_iopl(MachMode::Kernel)
        }
        pub fn kdata64() -> Descr {
            Self::empty()
                .with_system(true)
                .with_code(false)
                .with_readable(true)
                .with_present(true)
                .with_long(true)
                .with_iopl(MachMode::Kernel)
        }
        pub fn utext64() -> Descr {
            Self::empty()
                .with_system(true)
                .with_code(true)
                .with_readable(true)
                .with_present(true)
                .with_long(true)
                .with_iopl(MachMode::User)
        }
        pub fn udata64() -> Descr {
            Self::empty()
                .with_system(true)
                .with_code(false)
                .with_readable(true)
                .with_present(true)
                .with_iopl(MachMode::User)
        }
    }

    bitstruct! {
        /// Interrupt Gate Descriptors are entries in the IDT.
        #[derive(Clone, Copy, Default, FromZeros)]
        #[repr(transparent)]
        pub struct IntrGateDescr(u128) {
            pub offset0: u16 = 0..16;
            pub segment_selector: u16 = 16..32;
            pub stack_table_index: u8 = 32..35;
            mbz0: bool = 35;
            mbz1: bool = 36;
            mbz2: u8 = 37..40;
            fixed_type: u8 = 40..44;
            mbz3: bool = 44;
            dpl: MachMode = 45..47;
            pub present: bool = 47;
            pub offset16: u16 = 48..64;
            pub offset32: u32 = 64..96;
            pub reserved0: u32 = 96..128;
        }
    }

    impl IntrGateDescr {
        pub const fn empty() -> IntrGateDescr {
            const TYPE_INTERRUPT_GATE: u128 = 0b1110 << (32 + 8);
            IntrGateDescr(TYPE_INTERRUPT_GATE)
        }

        pub fn new(thunk: &trap::Stub, dpl: MachMode, stack_index: IstIndex) -> IntrGateDescr {
            let ptr: *const trap::Stub = thunk;
            let va = ptr.addr();
            IntrGateDescr::empty()
                .with_offset0(va.get_bits(0..16) as u16)
                .with_offset16(va.get_bits(16..32) as u16)
                .with_offset32(va.get_bits(32..64) as u32)
                .with_stack_table_index(stack_index as u8)
                .with_segment_selector(Gdt::ktextsel())
                .with_present(true)
                .with_dpl(dpl)
        }
    }

    bitstruct! {
        /// The Task State Descriptor provides the hardware with sufficient
        /// information to locate the TSS in memory.  The TSS, in turn,
        /// mostly holds stack pointers.
        #[derive(Clone, Copy, Debug, FromZeros)]
        #[repr(transparent)]
        pub struct TaskStateDescr(u128) {
            pub limit0: u16 = 0..16;
            pub base0: u16 = 16..32;
            pub base16: u8 = 32..40;
            mbo0: bool = 40;
            pub busy: bool = 41;
            mbz0: bool = 42;
            mbo1: bool = 43;
            mbz1: bool = 44;
            cpl: MachMode = 45..47;
            pub present: bool = 47;
            pub limit16: u8 = 48..52;
            pub avl: bool = 52;
            mbz2: bool = 53;
            mbz3: bool = 54;
            pub granularity: bool = 55;
            pub base24: u8 = 56..64;
            pub base32: u32 = 64..96;
            reserved0: u8 = 96..104;
            mbz4: u8 = 104..108;
            reserved1: u32 = 108..128;
        }
    }

    impl TaskStateDescr {
        pub const fn empty() -> Self {
            Self(0)
        }

        pub(super) fn new(tss: *mut Tss) -> Self {
            let ptr = tss.cast_const();
            let va = ptr.addr() as u64;
            Self::empty()
                .with_limit0(core::mem::size_of::<Tss>() as u16 - 1)
                .with_base0(va.get_bits(0..16) as u16)
                .with_base16(va.get_bits(16..24) as u8)
                .with_mbo0(true)
                .with_mbo1(true)
                .with_cpl(MachMode::Kernel)
                .with_present(true)
                .with_avl(true)
                .with_granularity(true)
                .with_base24(va.get_bits(24..32) as u8)
                .with_base32(va.get_bits(32..64) as u32)
        }
    }
}

#[derive(FromZeros, Debug)]
#[repr(C)]
struct GdtData {
    null: seg::Descr,
    ktext: seg::Descr,
    kdata: seg::Descr,
    udata: seg::Descr,
    utext: seg::Descr,
    unused: seg::Descr,
    task: seg::TaskStateDescr,
}

impl GdtData {
    fn init_in(page: &mut Page, tss: *mut Tss) {
        let ptr = page.as_mut_ptr() as *mut Self;
        let gdt = unsafe { &mut *ptr };
        gdt.init(tss);
    }

    pub fn init(&mut self, tss: *mut Tss) {
        self.null = seg::Descr::null();
        self.ktext = seg::Descr::ktext64();
        self.kdata = seg::Descr::kdata64();
        self.udata = seg::Descr::udata64();
        self.utext = seg::Descr::utext64();
        self.unused = seg::Descr::empty();
        self.task = seg::TaskStateDescr::new(tss);
    }
}

#[derive(FromZeros)]
#[repr(C, align(65536))]
pub struct Gdt(GdtData);

impl Gdt {
    pub fn init_in(page: &mut Page, tss: *mut Tss) {
        GdtData::init_in(page, tss);
    }

    pub const fn ktextsel() -> u16 {
        core::mem::offset_of!(GdtData, ktext) as u16
    }
    pub const fn utextsel() -> u16 {
        core::mem::offset_of!(GdtData, utext) as u16
    }
    pub const fn tasksel() -> u16 {
        core::mem::offset_of!(GdtData, task) as u16
    }

    /// # Safety
    /// It is up to the caller to ensure that `self` has
    /// been properly initialized.
    pub unsafe fn load(&mut self) {
        unsafe { cpu::lgdt(self) }
    }

    // On 64-bit return, SYSRETQ loads the CS with
    // the value of star[48..64] + 16.  Why +16?
    // For compatibility with legacy 32-bit
    // systems: presumably the GDT would have 32-bit
    // entries first, then 64-bit entries, and SYSRET
    // returns to a 64-bit segment.
    //
    // On a 64-bit only system, this isn't an issue,
    // but we still need to ensure the value in IA32_STAR
    // at this offset is properly loaded.
    //
    // Interestingly, SS is loaded with star[48..64] + 8.
    // In R9, this is the user data selector, which is
    // exactly what we want.
    //
    // Note that SYSRET on Intel forces RPL on the
    // loaded selectors to 3, but AMD only does this
    // for CS and not SS.  So for portability, we
    // explicitly OR a user RPL into the STAR bits.
    // This is idempotent for CS.
    pub fn star() -> u64 {
        const RPL_USER: u64 = MachMode::User as u64;
        const UTEXTSEL: u64 = Gdt::utextsel() as u64;
        const KTEXTSEL: u64 = Gdt::ktextsel() as u64;
        ((UTEXTSEL - 16) | RPL_USER) << 48 | KTEXTSEL << 32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IstIndex {
    Rsp0 = 0,
    Ist1 = 1,
    Ist2 = 2,
    Ist3 = 3,
    Ist4 = 4,
    Ist5 = 5,
    Ist6 = 6,
    Ist7 = 7,
}

impl IstIndex {
    fn from_trap(trapno: u8) -> Self {
        match trapno {
            NMI_TRAPNO => IstIndex::Ist1,
            DEBUG_TRAPNO => IstIndex::Ist2,
            BREAKPOINT_TRAPNO => IstIndex::Ist3,
            DOUBLE_FAULT_TRAPNO => IstIndex::Ist4,
            _ => IstIndex::Rsp0,
        }
    }
}

#[derive(FromZeros)]
#[repr(C)]
pub struct Tss {
    _res0: u32,
    rsp0: [u32; 2],
    _rsp1: [u32; 2],
    _rsp2: [u32; 2],
    _res1: u32,
    ist1: [u32; 2],
    ist2: [u32; 2],
    ist3: [u32; 2],
    ist4: [u32; 2],
    ist5: [u32; 2],
    ist6: [u32; 2],
    ist7: [u32; 2],
    _res3: u32,
    _res4: u32,
    _res5: u16,
    iomb: u16,
}

impl Tss {
    pub fn init(&mut self, stacks: &mut [(u8, &mut dyn Stack)]) {
        stacks.iter_mut().for_each(|(trapno, stack)| self.set_stack(*trapno, *stack));
        self.iomb = core::mem::size_of::<Tss>() as u16;
    }

    pub fn set_rsp0(&mut self, stack: &mut dyn Stack) {
        self.set_stack(0, stack);
    }

    fn set_stack(&mut self, trapno: u8, stack: &mut dyn Stack) {
        let index = IstIndex::from_trap(trapno);
        let va = stack.top_mut().addr();
        let lower = va.get_bits(0..32) as u32;
        let upper = va.get_bits(32..64) as u32;
        match index {
            IstIndex::Rsp0 => self.rsp0 = [lower, upper],
            IstIndex::Ist1 => self.ist1 = [lower, upper],
            IstIndex::Ist2 => self.ist2 = [lower, upper],
            IstIndex::Ist3 => self.ist3 = [lower, upper],
            IstIndex::Ist4 => self.ist4 = [lower, upper],
            IstIndex::Ist5 => self.ist5 = [lower, upper],
            IstIndex::Ist6 => self.ist6 = [lower, upper],
            IstIndex::Ist7 => self.ist7 = [lower, upper],
        }
    }

    /// # Safety
    /// It is up to the caller to ensure that an appropriate GDT
    /// is initialized and loaded, and that this TSS has been
    /// initialized.
    pub unsafe fn load(&mut self) {
        unsafe {
            cpu::ltr(Gdt::tasksel());
        }
    }
}

#[derive(FromZeros)]
#[repr(C, align(4096))]
pub struct Idt([seg::IntrGateDescr; 256]);

impl Idt {
    pub const fn empty() -> Self {
        Idt([seg::IntrGateDescr::empty(); 256])
    }

    pub fn init(&mut self, stubs: &[trap::Stub; 256]) {
        for (k, thunk) in stubs.iter().enumerate() {
            let trapno = k as u8;
            let index = IstIndex::from_trap(trapno);
            let dpl = if trapno == BREAKPOINT_TRAPNO { MachMode::User } else { MachMode::Kernel };
            self.0[k] = seg::IntrGateDescr::new(thunk, dpl, index);
        }
    }

    /// # Safety
    /// It is up to the caller to ensure that this IDT has been
    /// properly initialized.
    pub unsafe fn load(&mut self) {
        unsafe {
            cpu::lidt(self);
        }
    }
}

pub unsafe fn init(mach: &mut Mach) {
    const MSR_KERNEL_GS_BASE: u32 = 0xc0000102;
    unsafe {
        mach.init();
        let ptr = mach as *mut Mach;
        let me = ptr.addr() + 0x002_0000;
        cpu::wrgsbase(me as u64);
        cpu::wrmsr(MSR_KERNEL_GS_BASE, 0);
    }
}
