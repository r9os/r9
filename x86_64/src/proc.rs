pub use crate::dat::Label;
use core::arch::naked_asm;

#[unsafe(naked)]
pub unsafe extern "C" fn swtch(save: &mut Label, next: &mut Label) {
    naked_asm!(
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
        options(att_syntax)
    );
}
