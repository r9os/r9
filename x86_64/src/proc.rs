use core::arch::asm;

#[repr(C)]
pub struct Label {
    pub pc: u64,
    pub sp: u64,
    pub fp: u64,
    rbx: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

impl Label {
    pub const fn new() -> Label {
        Label { pc: 0, sp: 0, fp: 0, rbx: 0, r12: 0, r13: 0, r14: 0, r15: 0 }
    }
}

#[naked]
pub unsafe extern "C" fn swtch(save: &mut Label, next: &mut Label) {
    unsafe {
        asm!(
            r#"
            movq (%rsp), %rax
            movq %rax, 0(%rdi)
            movq %rsp, 8(%rdi)
            movq %rbp, 16(%rdi)
            movq %rbx, 24(%rdi)
            movq %r12, 32(%rdi)
            movq %r13, 40(%rdi)
            movq %r14, 48(%rdi)
            movq %r15, 56(%rdi)

            movq 0(%rsi), %rax
            movq 8(%rsi), %rsp
            movq 16(%rsi), %rbp
            movq 24(%rsi), %rbx
            movq 32(%rsi), %r12
            movq 40(%rsi), %r13
            movq 48(%rsi), %r14
            movq 56(%rsi), %r15
            movq %rax, (%rsp)
            xorl %eax, %eax
            ret"#,
            options(att_syntax, noreturn)
        );
    }
}
