use crate::cpu;
use crate::dat;
use crate::dat::{UREG_CS_OFFSET, UREG_TRAPNO_OFFSET, Ureg};

use core::arch::naked_asm;

pub const DEBUG_TRAPNO: u8 = 1;
pub const NMI_TRAPNO: u8 = 2;
pub const BREAKPOINT_TRAPNO: u8 = 3;
pub const DOUBLE_FAULT_TRAPNO: u8 = 8;

pub const SWAPPED: u8 = 0x80;
pub const SWAPPED_CS_OFFSET: usize = 7;

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
        r#".balign 8; pushq $0; callq {intrregular};"#
    };
    (spec) => {
        r#".balign 8; pushq $0; callq {intrspecial};"#
    };
    (err) => {
        r#".balign 8; callq {intrregular};"#
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
    // These interrupts are special, in that GSBASE may not be properly
    // set when we enter them.
    (1) => {
        gen_stub!(spec)
    };
    (2) => {
        gen_stub!(spec)
    };
    (3) => {
        gen_stub!(spec)
    };
    (8) => {
        gen_stub!(spec)
    };
    // No hardware error
    ($num:literal) => {
        gen_stub!()
    };
}

pub fn stubs() -> &'static [Stub; 256] {
    unsafe { &*(intr_stubs as *const () as usize as *const [Stub; 256]) }
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
pub unsafe extern "C" fn intr_stubs() {
    use seq_macro::seq;
    naked_asm!(
        seq!(N in 0..=255 {
            concat!( #( gen_trap_stub!(N), )* )
        }),
        intrregular = sym intrregular,
        intrspecial = sym intrspecial,
        options(att_syntax))
}

/// Some interrupts require special handling on delivery,
/// because there is no good way to detect whether `GSBASE` was
/// swapped with the contenst of `MSR_KERNEL_GS_BASE` by simply
/// looking at the exception frame on the stack: NMIs, double
/// faults, and the various debug exceptions fall into this
/// category.
///
/// To understand this, consider an NMI and how it may interact
/// with a non-NMI interrupt handler that is executing in kernel
/// mode: on receipt of an interrupt, the CPU will look up the
/// corresponding IDT entry and invoke a gate handler for the
/// interrupt.  On R9, the handler from the interrupt gate
/// defined in the IDT is one of the trap stubs created above,
/// and the trap stub will be invoked with interrupts disabled.
///
/// The trap stub ensures the stack is aligned vis the error
/// number and then immediately call into `intrcommon`, which
/// saves registers and then examines the CS segment selector
/// pushed onto stack by hardware.  If we're already in the
/// kernel, we just continue executing.  If not, then we
/// execute a `SWAPGS` instruction, so that `%GSBASE` points to
/// this CPU's `Mach` and we have access to the (kernel's)
/// per-CPU data.
///
/// While the interrupt gate prohibits delivery of other regular
/// interrupts, this does not apply to either synchronous
/// exceptions or to NMIs, which by definition are not maskable.
/// Since we are in a trap handler, any non-debug synchronous
/// exception will result in a double fault, and NMIs are
/// delivered normally.  (An exception is that, when handling
/// an NMI, delivery of subsequent NMIs is suspended until an
/// `IRETQ` is executed, but here we're considering the case of
/// NMI delivery while handling a maskable interrupt).
///
/// Thus, there is a race here: when handling a normal interrupt
/// or synchronous exception, an NMI may be delivered any time
/// after we have entered the kernel, but before we have
/// executed `SWAPGS`.  In that case, if we entered via the
/// normal interrupt handling path, we would see that we were
/// already in kernel mode and elide the `SWAPGS`, so that when
/// we entered the rest of the kernel, `%GSBASE` would still be
/// pointing to whatever it had been set to while running in
/// user mode, which is clearly wrong.
///
/// To address this, when we receive one of these traps, we need
/// to decide whether to invoke `SWAPGS` by testing something
/// more reliable than the code segment selector on the stack.
/// Fortunately on R9, the Mach is at a fixed virtual address
/// for each CPU, so we can look at `%GSBASE` itself (e.g., via
/// `RDGSBASE`) to see if it is already pointing to our `Mach`,
/// but is this safe?   There are two cases: if it is not equal
/// to our Mach, then we need to `SWAPGS`.  If it is pointing
/// to our Mach, then one of two things has happened: either
/// we have already executed `SWAPGS`, and whatever userspace
/// had set `%GSBASE` to is in `MSR_KERNEL_GS_BASE`, or we have
/// not executed `SWAPGS` and userspace had explicitly set
/// `%GSBASE` to the address of our `Mach`.  In the latter case,
/// `MSR_KERNEL_GS_BASE` will still hold the address of our
/// `Mach`, and so whether or not we run `SWAPGS` is moot.
///
/// There is also a separate, related, complication we must
/// handle in the return path.  When returning to userspace, we
/// want to make sure that we run `SWAPGS` again to restore the
/// user `%GSBASE` (and stash our `Mach` pointer in
/// `MSR_KERNEL_GS_BASE`).  But if we simply test the code
/// segment selector on the stack frame as a proxy for this, we
/// open ourselves up to the same race as when deciding whether
/// to invoke `SWAPGS` on entry or not.  Specifically, consider
/// the case of running in user mode when an ordinary interrupt
/// is received.  We test against the code segment selector as a
/// proxy for whether we should `SWAPGS`.  Now suppose that an
/// NMI is delivered after testing the CS selector, but before
/// `SWAPGS`: the NMI handler will correctly `SWAPGS` itself,
/// but when returning from the NMI, we would see that we came
/// from kernel mode and skip swapping when we go back to the
/// original interrupt handler.  But that handler had already
/// observed that we were coming from userspace, and should
/// will now execute `SWAPGS`, which will undo what the NMI
/// handler had done: we will now be running in the kernel with
/// the user `%GSBASE`.
///
/// To avoid this, we again need a more robust mechanism to
/// determine whether we should `SWAPGS` on return: in effect we
/// need some kind of witness to whether this invocation of the
/// handler swapped, and our invariant is that, if `%GSBASE` was
/// swapped on entry, it must be swapped on return.
///
/// To handle this, we need some place to stash knowledge that
/// we ran `SWAPGS`.  Fortunately, the 16-bit code segment
/// selector is zero-extended and pushed to the trap stack as a
/// 64-bit word, so we know that there are 48 zero bits at a
/// known location on the stack.  If we execute `SWAPGS`, we
/// set one of those bits, and test it on return; if set, we
/// `SWAPGS` in the return path.
///
/// Returning to the example of an NMI received after entry to
/// a handler due to an interrupt received while in user mode,
/// and after the comparison against the code segment selectors
/// but before `SWAPGS`, the NMI handler we will observe that
/// `%GSBASE` needs to be swapped and will `SWAPGS` and set a
/// bit in the high part of the CS selector on the stack.  On
/// return, it will see that the bit is set and `SWAPGS` again,
/// restoring the user `%GSBASE`.  Now back in the original
/// interrupt handler, we will `SWAPGS`, again properly setting
/// `%GSBASE` and setting a bit to restore it on return;
/// assuming no other intervening NMIs or synchronous exceptions
/// are delivered, we will observe the bit set in the segment
/// selector word on the stack, `SWAPGS` to restore the user
/// `%GSBASE`, clear the bit, and return normally.
///
/// # Safety
///
/// Invoked directly from trap stubs.
#[unsafe(link_section = ".trap")]
#[unsafe(naked)]
pub unsafe extern "C" fn intrspecial() {
    naked_asm!(r#"
        pushq %rax;
        pushq %rbx;
        movabsq ${KMACH}, %rbx;
        rdgsbase %rax;
        cmpq %rbx, %rax;
        popq %rbx;
        popq %rax;
        je 1f;
        swapgs;
        orb ${SWAPPED}, {SWAPPED_CS_OFFSET}(%rsp)
        1:
        pushq ${intrcommon};
        retq;
        "#,
        KMACH = const dat::KMACH,
        SWAPPED = const SWAPPED,
        SWAPPED_CS_OFFSET = const SWAPPED_CS_OFFSET + 16,
        intrcommon = sym intrcommon,
        options(att_syntax))
}

/// The "regular" interrupt entry point.  It does `SWAPGS` if
/// necessary, based on the code segment selector pushed onto
/// the stack by hardware.  See the comments at the top of
/// `intrspecial` to understand what is going on here.
///
/// # Safety
///
/// Regular trap handler, called from (most) trap stubs.
#[unsafe(link_section = ".trap")]
#[unsafe(naked)]
unsafe extern "C" fn intrregular() {
    naked_asm!(r#"
        // Check if we need to execute `SWAPGS`.
        // Skip if we're already in kernel mode.
        cmpw ${KTEXT_SEL}, {CS_OFFSET}(%rsp);
        je 1f;
        swapgs;
        orb ${SWAPPED}, {SWAPPED_CS_OFFSET}(%rsp)
        1:
        jmp {intrcommon}
        "#,
        KTEXT_SEL = const 8,
        SWAPPED = const SWAPPED,
        SWAPPED_CS_OFFSET = const SWAPPED_CS_OFFSET + 24,
        CS_OFFSET = const 24,
        intrcommon = sym intrcommon,
        options(att_syntax));
}

/// intrcommon builds up a Ureg structure at the top of the
/// current proc's kernel stack and invokes the main trap
/// entry point.
///
/// See the icomments on `intrspecial` about return handling for
/// `SWAPGS` to understand `SWAPPED` etc.
///
/// # Safety
///
/// Common trap handler.  Called from interrupt/exception stub.
#[unsafe(link_section = ".trap")]
#[unsafe(naked)]
unsafe extern "C" fn intrcommon() {
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
        shrw $3, {TRAPNO_OFFSET}(%rsp);
        movzbl {TRAPNO_OFFSET}(%rsp), %edi;
        movq %rdi, {TRAPNO_OFFSET}(%rsp);
        movq %rsp, %rsi;
        callq {trap};
        // If we're returning to kernel mode, don't swap %gs.
        btr ${SWAPPED}, {SWAPPED_CS_OFFSET}(%rsp);
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
        SWAPPED = const SWAPPED,
        SWAPPED_CS_OFFSET = const SWAPPED_CS_OFFSET + UREG_CS_OFFSET,
        TRAPNO_OFFSET = const UREG_TRAPNO_OFFSET,
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
    crate::println!("trap {vector:?}", vector = crate::dat::Trap::from(vector));
    match vector {
        2 => {}
        8 => unsafe { core::arch::asm!("hlt") },
        _ => crate::println!("frame: {trap_frame:#x?}"),
    }
    0
}
