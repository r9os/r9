use crate::cpu;
use crate::dat::Ureg;
use crate::dat::{UREG_CS_OFFSET, UREG_TRAPNO_OFFSET};

use core::arch::naked_asm;

pub const DEBUG_TRAPNO: u8 = 1;
pub const NMI_TRAPNO: u8 = 2;
pub const DOUBLE_FAULT_TRAPNO: u8 = 8;

type Thunk = unsafe extern "C" fn();

#[repr(transparent)]
pub struct Stub(usize);

impl Stub {
    /// Returns a function pointer that represents a stub.
    pub unsafe fn as_thunk(&self) -> Thunk {
        unsafe { core::mem::transmute::<usize, Thunk>(self.0) }
    }
}

macro_rules! gen_stub {
    () => {
        r#".balign 8; pushq $0; callq {intrcommon};"#
    };
    (err) => {
        r#".balign 8; callq {intrcommon};"#
    };
}

macro_rules! gen_trap_stub {
    // These cases include hardware-generated error words
    // on the trap frame
    (8) => {
        gen_stub!(err)
    };
    (10) => {
        gen_stub!(err)
    };
    (11) => {
        gen_stub!(err)
    };
    (12) => {
        gen_stub!(err)
    };
    (13) => {
        gen_stub!(err)
    };
    (14) => {
        gen_stub!(err)
    };
    (17) => {
        gen_stub!(err)
    };
    // No hardware error
    ($num:literal) => {
        gen_stub!()
    };
}

pub fn stubs() -> &'static [Stub; 256] {
    unsafe { &*(intr_stubs as usize as *const [Stub; 256]) }
}

/// intr_stubs is just a container for interrupt stubs.  It is
/// a naked function for convenience.
///
/// # Safety
///
/// Container for thunks.
#[unsafe(link_section = ".trap")]
#[unsafe(naked)]
#[rustc_align(4096)]
pub unsafe extern "C" fn intr_stubs() -> ! {
    use seq_macro::seq;
    naked_asm!(
        seq!(N in 0..=255 {
            concat!( #( gen_trap_stub!(N), )* )
        }),
        intrcommon = sym intrcommon, options(att_syntax))
}

/// intrcommon builds up a Ureg structure at the top of the
/// current proc's kernel stack.
///
/// # Safety
///
/// Common trap handler.  Called from interrupt/exception stub.
#[unsafe(link_section = ".trap")]
#[unsafe(naked)]
pub unsafe extern "C" fn intrcommon() -> ! {
    naked_asm!(r#"
        // Allocate space to save registers.
        subq $((4 + 15) * 8), %rsp
        // Save the general purpose registers.
        movq %r15, 14*8(%rsp);
        movq %r14, 13*8(%rsp);
        movq %r13, 12*8(%rsp);
        movq %r12, 11*8(%rsp);
        movq %r11, 10*8(%rsp);
        movq %r10, 9*8(%rsp);
        movq %r9, 8*8(%rsp);
        movq %r8, 7*8(%rsp);
        movq %rbp, 6*8(%rsp);
        movq %rdi, 5*8(%rsp);
        movq %rsi, 4*8(%rsp);
        movq %rdx, 3*8(%rsp);
        movq %rcx, 2*8(%rsp);
        movq %rbx, 1*8(%rsp);
        movq %rax, 0*8(%rsp);
        // Save the x86 segmentation registers.  Uses %rdi
        // as a scratch register, so we do this after we've
        // saved the GP registers..  Note that the 32-bit
        // `movl` zero-extends the segmentation register and
        // clears the upper bits of %rdi.  We use this
        // because the result has a denser encoding than
        // other instruction sequences.
        movl %gs, %edi;
        movq %rdi, 18*8(%rsp);
        movl %fs, %edi;
        movq %rdi, 17*8(%rsp);
        movl %es, %edi;
        movq %rdi, 16*8(%rsp);
        movl %ds, %edi;
        movq %rdi, 15*8(%rsp);
        // Fix up the trap number.  We got here via a CALL,
        // so hardware pushed the address after the CALLQ
        // instruction onto the stack.  But we know that
        // each stub is aligned to an 8-byte boundary, at
        // some offset based on the vector number relative
        // to the 4096-byte aligned start of the trap stub
        // array.  Further, each stub is shorter than 8
        // bytes in length.  Thus, we can compute the
        // vector number by dividing the return address by
        // 8, masking off the high bits, and storing it back
        // into the save area.
        //
        // The vector number is an argument to the trap
        // function, along with the address of the Ureg
        // we have built at the top of the stack.
        shrw $3, {trapno_offset}(%rsp);
        movzbl {trapno_offset}(%rsp), %edi;
        movq %rdi, {trapno_offset}(%rsp);
        movq %rsp, %rsi;
        // If we're already in kernel mode, don't swap %gs.
        cmpq ${ktext_sel}, {cs_offset}(%rsp);
        je 1f;
        swapgs;
        1:
        callq {trap};
        // If we're returning to kernel mode, don't swap %gs.
        cmpq ${ktext_sel}, {cs_offset}(%rsp);
        je 1f;
        swapgs;
        1:
        // Restore the general purpose registers.
        movq 0*8(%rsp), %rax;
        movq 1*8(%rsp), %rbx;
        movq 2*8(%rsp), %rcx;
        movq 3*8(%rsp), %rdx;
        movq 4*8(%rsp), %rsi;
        movq 5*8(%rsp), %rdi;
        movq 6*8(%rsp), %rbp;
        movq 7*8(%rsp), %r8;
        movq 8*8(%rsp), %r9;
        movq 9*8(%rsp), %r10;
        movq 10*8(%rsp), %r11;
        movq 11*8(%rsp), %r12;
        movq 12*8(%rsp), %r13;
        movq 13*8(%rsp), %r14;
        movq 14*8(%rsp), %r15;
        // Restore the segmentation registers.
        movw 15*8(%rsp), %ds;
        movw 16*8(%rsp), %es;
        // %gs is restored via swapgs above.  The system never changes
        // it, so we don't bother restoring it here.  %fs is special.
        // We do save and restore it, for TLS if anyone ever uses green
        // threads.
        movw 17*8(%rsp), %fs;
        // movw 18*8(%rsp), %gs;
        // Pop registers, alignment word and error.
        addq $((2 + 4 + 15) * 8), %rsp;
        // Go back to whence you came.
        iretq
        "#,
        ktext_sel = const 8,
        cs_offset = const UREG_CS_OFFSET,
        trapno_offset = const UREG_TRAPNO_OFFSET,
        trap = sym trap,
        options(att_syntax))
}

pub enum IntrStatus {
    Disabled = 0,
    Enabled = 1,
}

#[inline(always)]
fn intrstatus() -> IntrStatus {
    let flags = cpu::flags();
    if flags.intr() { IntrStatus::Enabled } else { IntrStatus::Disabled }
}

pub fn spllo() -> IntrStatus {
    let prev_level = intrstatus();
    cpu::sti();
    prev_level
}

pub fn splhi() -> IntrStatus {
    let prev_level = intrstatus();
    cpu::cli();
    prev_level
}

pub fn splx(x: IntrStatus) -> IntrStatus {
    match x {
        IntrStatus::Disabled => splhi(),
        IntrStatus::Enabled => spllo(),
    }
}

extern "C" fn trap(vector: u8, trap_frame: &mut Ureg) -> u32 {
    crate::println!("trap {vector}");
    crate::println!("frame: {trap_frame:#x?}");
    unsafe { core::arch::asm!("cli;hlt;") };
    0
}
