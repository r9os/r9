.section .text.entry
.globl start
start:
	bnez	a0, 1f
	la	sp, stack	// set the stack pointer
	li	t0, 4096 * 4
	add	sp, sp, t0	// add stack length
	call	main9
1:
	wfi
	j	1b

.bss
.balign 4096
stack:	.space 4096 * 4
