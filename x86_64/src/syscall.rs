//! Syscall support code.

use crate::cpu;
use crate::dat;
use crate::vsvm;

use core::arch::naked_asm;

pub(crate) fn init() {
    const MSR_STAR: u32 = 0xc000_0081;
    const MSR_LSTAR: u32 = 0xc000_0082;
    const MSR_FMASK: u32 = 0xc000_0084;
    unsafe {
        cpu::wrmsr(MSR_LSTAR, entry as usize as u64);
        cpu::wrmsr(MSR_STAR, vsvm::Gdt::star());
        cpu::wrmsr(MSR_FMASK, cpu::fmask());
    }
}

extern "C" fn dispatch(user: &mut dat::Ureg, sysno: u32) -> i64 {
    crate::println!("Got a system call ({sysno}): {user:#x?}");
    0
}

/// This is the system call entry handler, that is invoked by
/// the execution of the `SYSCALL` instruction.
///
/// On entry:
///  - The user %rip is in %rcx (hardware)
///  - The user %rflags is in %r11 (hardware)
///  - System call number is in %rax (software convention)
///  - Up to 6 system call arguments are passed in %rdi, %rsi,
///    %rdx, %r10, %r8, and $9, respectively.
///
/// None of the other general purpose registers are handled
/// specially.
#[unsafe(naked)]
unsafe extern "C" fn entry() {
    naked_asm!(
        r#"
        // Switch user and kernel GSBASE
        swapgs

        // Stash the user stack pointer in the Mach's
        // `scratch` field, and set the kernel stack
        // pointer.  The kernel stack is known to be
        // just below the proc.
        movq	%rsp, %gs:8
        movq	%gs:24, %rsp

        // We construct a Ureg on the stack, but many of the
        // fields therein are not used by the system call machinery.
        // We push dummy values for them anyway.
        subq    $(26*8), %rsp       // mem::size_of::<dat::Ureg>()

        movq    %rax, 0*8(%rsp)     // ureg.ax  (syscall number)
        movq    %rbx, 1*8(%rsp)     // ureg.bx
        movq    %rcx, 2*8(%rsp)     // ureg.cx  (user pc)
        movq    %rdx, 3*8(%rsp)     // ureg.dx
        movq    %rsi, 4*8(%rsp)     // ureg.si
        movq    %rdi, 5*8(%rsp)     // ureg.di
        movq    %rbp, 6*8(%rsp)     // ureg.bp
        movq    %r8, 7*8(%rsp)      // ureg.r8
        movq    %r9, 8*8(%rsp)      // ureg.r9
        movq    %r10, 9*8(%rsp)     // ureg.r10
        movq    %r11, 10*8(%rsp)    // ureg.r11 (user flags)
        movq    %r12, 11*8(%rsp)    // ureg.r12
        movq    %r13, 12*8(%rsp)    // ureg.r13
        movq    %r14, 13*8(%rsp)    // ureg.r14
        movq    %r15, 14*8(%rsp)    // ureg.r15
        // Save the x86 segmentation registers.  Uses %rdi
        // as a scratch register, so we do this after we've
        // saved the GP registers.  Note that the 32-bit
        // `movl` zero-extends the segmentation register and
        // clears the upper bits of %rdi.  We use this
        // because the result has a denser encoding than
        // other instruction sequences.
        movl	%ds, %ebx
        movq	%rbx, 15*8(%rsp)    // ureg.ds
        movl	%es, %ebx
        movq	%rbx, 16*8(%rsp)    // ureg.es
        movl	%fs, %ebx
        movq	%rbx, 17*8(%rsp)    // ureg.fs
        // %gs is special due to SWAPGS
        movq	$0, 18*8(%rsp)      // ureg.gs

        movq    $0, 19*8(%rsp)      // ureg.trapno
        movq    $0, 20*8(%rsp)      // ureg.ecode
        movq    %rcx, 21*8(%rsp)    // ureg.pc

        movl	%cs, %ebx
        movq    %rbx, 22*8(%rsp)    // ureg.cs

        movq    %r11, 23*8(%rsp)    // ureg.flags

        movq    %gs:8, %rbx
        movq    %rbx, 24*8(%rsp)    // ureg.sp
        movl    %ss, %ebx
        movq    %rbx, 25*8(%rsp)    // ureg.ss

        // Set up a call frame so that we can get a back trace
        // from here, possibly into user code.
        pushq   ${syscallret}       // We'll return here.
        pushq   %rcx                // ret is user PC
        movq    %gs:8, %rbp         // user sp

        // System call number is 4th argument to `syscall` function.
        movq    %rsp, %rdi          // *mut Ureg is first argument
        movq    %rax, %rsi          // system call num is second arg

        // Call the handler in Rust.
        // XXX: Could we `sti` here?
        callq {syscall}

        // Pop dummy stack frame and jump to syscallret
        addq    $8, %rsp
        ret
        "#,
        syscall = sym dispatch,
        syscallret = sym ret,
        options(att_syntax)
    );
}

#[unsafe(naked)]
pub unsafe extern "C" fn ret() {
    naked_asm!(
        r#"
        // Skip %rax. It is the return value from the system call.
        addq $8, %rsp

        // %rax is the return value from the system call.
        movq    1*8(%rsp), %rbx
        // %rcx is handled specially
        movq    3*8(%rsp), %rdx
        movq    4*8(%rsp), %rsi
        movq    5*8(%rsp), %rdi
        movq    6*8(%rsp), %rbp
        movq    7*8(%rsp), %r8
        movq    8*8(%rsp), %r9
        movq    9*8(%rsp), %r10
        movq    10*8(%rsp), %r11
        movq    11*8(%rsp), %r12
        movq    12*8(%rsp), %r13
        movq    13*8(%rsp), %r14
        movq    14*8(%rsp), %r15

        movq    21*8(%rsp), %rcx    // user PC goes into %rcx
        movq    23*8(%rsp), %r11    // user FLAGS goes into %r11
        orq     $2, %r11            // MB1 bit

        // Once we start modifying segmentation and stack registers, we
        // want to ensure that interrupts are disabled, so that we do not
        // accidentally take an interrupt on a user stack.
        cli

        // Restore user segmentation registers.
        movw    15*8(%rsp), %ds
        movw    16*8(%rsp), %es
        movw    17*8(%rsp), %fs
        // %gs is specially restored by `swapgs`, below.

        // Restore user stack pointer.  Note that this
        // implicitly deallocates the Ureg; on the next
        // entry to the kernel, we will reset the stack
        // pointer to the top of the kstack anyway.  This
        // implies that the kernel stack for a proc is
        // empty while that program runs in user mode.
        movq    24*8(%rsp), %rsp
        // %ss will be reloaded automatically from STAR.

        // Switch kernel, user GSBASE
        swapgs

        // Return from system call
        sysretq
        "#,
        options(att_syntax)
    );
}
