//! Architecture specific bits.  Mostly interfaces to specific
//! machine registers or instructions that must be accessed from
//! assembler.

use crate::dat::{Flags, Gdt, Idt};

use bit_field::BitField;
use core::arch::asm;

/// Retrieves a copy of the `RFLAGS` registers.
pub(crate) fn flags() -> Flags {
    const MB1: u64 = 0b10;
    unsafe {
        let raw: u64;
        asm!("pushfq; popq {};", out(reg) raw, options(att_syntax));
        Flags::new(raw | MB1)
    }
}

pub(crate) fn fmask() -> u64 {
    Flags::empty().with_intr(true).with_trap(true).with_dir(true).bits()
}

/// Executes the `STI` instruction that enables interrupt
/// delivery on the current CPU, by setting the "Interrupt
/// Enable" bit (`IF`) in the `RFLAGS` register
pub(crate) fn sti() {
    unsafe {
        asm!("sti");
    }
}

/// Executes the `CLI` instruction that disables interrupt
/// delivery on the current CPU, by clearing the "Interrupt
/// Enable" bit (`IF`) in the `RFLAGS` register
pub(crate) fn cli() {
    unsafe {
        asm!("cli");
    }
}

/// Loads the "Task Register" (`TR`) with the given 16-bit
/// selector index, which identifies a "Task State Selector"
/// (that points to a "Task State Segment" [TSS]) in the Global
/// Descriptor Table (GDT).
///
/// # Safety
/// The given selector must identify a well-formed TSS selector
/// in the presently loaded GDT.
pub(crate) unsafe fn ltr(selector: u16) {
    unsafe {
        asm!("ltr {:x};", in(reg) selector);
    }
}

/// Loads the "Global Table Descriptor Register" (`GDTR`) with
/// the base address and inclusive limit of a "Global Descriptor
/// Table" (GDT).
///
/// # Safety
/// The referred GDT must be architecturally valid.
pub(crate) unsafe fn lgdt(gdt: &Gdt) {
    let ptr: *const Gdt = gdt;
    unsafe {
        asm!(r#"
            subq $16, %rsp;
            movq {base}, 8(%rsp);
            movw ${limit}, 6(%rsp)
            lgdt 6(%rsp);
            movq $8, 8(%rsp);
            lea 1f(%rip), %rax;
            movq %rax, (%rsp);
            lretq;
            1:
            "#,
            base = in(reg) u64::try_from(ptr.addr()).unwrap(),
            limit = const core::mem::size_of::<Gdt>().wrapping_sub(1) as u16,
            options(att_syntax)
        );
    }
}

/// Loads the "Interrupt Descriptor Table Register" (`IDTR`)
/// with the base address and inclusive limit of an "Interrupt
/// Descriptor Table" (IDT).
///
/// # Safety
/// The referred IDT must be architecturally valid.
pub(crate) unsafe fn lidt(idt: &Idt) {
    let ptr: *const Idt = idt;
    unsafe {
        asm!(r#"
            subq $16, %rsp;
            movq {base}, 8(%rsp);
            movw ${limit}, 6(%rsp)
            lidt 6(%rsp);
            addq $16, %rsp;
            "#,
            base = in(reg) u64::try_from(ptr.addr()).unwrap(),
            limit = const core::mem::size_of::<Idt>().wrapping_sub(1) as u16,
            options(att_syntax)
        );
    }
}

/// Reads an MSR
///
/// # Safety
/// The caller must ensure that the MSR is valid.
pub unsafe fn _rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!("rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
            options(att_syntax));
    }
    u64::from(hi) << 32 | u64::from(lo)
}

/// Writes an MSR
///
/// # Safety
/// The caller must ensure that the value and MSR are
/// valid, and that the value makes sense for the MSR.
pub(crate) unsafe fn wrmsr(msr: u32, value: u64) {
    unsafe {
        asm!("wrmsr",
            in("ecx") msr,
            in("eax") value.get_bits(0..32),
            in("edx") value.get_bits(32..64),
            options(att_syntax));
    }
}

/// Writes the GS Base register.
///
/// # Safety
/// The caller must ensure that the given value makes sense as
/// a %gs segment base value.  Note that we assume we can use
/// the `WRGSBASE` instruction.
pub(crate) unsafe fn wrgsbase(value: u64) {
    unsafe {
        asm!("wrgsbase {}", in(reg) value, options(att_syntax));
    }
}
