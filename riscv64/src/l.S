	.section .text.entry
	.globl _start
_start:
	la    sp, {stack}  // set the stack pointer
	li    t0, {len_per_hart} // stack length per hart
	addi  t1, a0, 1
1:  add   sp, sp, t0 // add stack length
	addi  t1, t1, -1
	bnez  t1, 1b

	call  main9
