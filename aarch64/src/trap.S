// Constants from trap.rs: KSTACK_SHIFT, log2 of KSTACKSIZE.  This file is a
// global_asm! format string, so literal braces must be doubled.

// The stack exceptions move to when a kernel stack overflows.  Like a kernel
// stack, it's the upper half of a block aligned to twice its size, so an sp
// inside it has bit KSTACK_SHIFT set, and the overflow check passes on it.
// There's one, for the one CPU r9 runs on: SMP needs one per CPU.
.bss
.balign	2 << {KSTACK_SHIFT}
.space	1 << {KSTACK_SHIFT}
.globl	overflowstackbase
overflowstackbase:	.space 1 << {KSTACK_SHIFT}
// A label rather than an absolute symbol for the size: Rust reaches extern
// statics through the GOT, which can't hold an absolute value when this and
// the code using it are assembled into one object.
.globl	overflowstacktop
overflowstacktop:

.section .text

.equ	SYNC_INVALID_EL1t,	0
.equ	IRQ_INVALID_EL1t,	1
.equ	FIQ_INVALID_EL1t,	2
.equ	ERROR_INVALID_EL1t,	3

.equ	SYNC_INVALID_EL1h,	4
.equ	IRQ_INVALID_EL1h,	5
.equ	FIQ_INVALID_EL1h,	6
.equ	ERROR_INVALID_EL1h,	7

.equ	SYNC_INVALID_EL0_64,	8
.equ	IRQ_INVALID_EL0_64,	9
.equ	FIQ_INVALID_EL0_64,	10
.equ	ERROR_INVALID_EL0_64,	11

.equ	SYNC_INVALID_EL0_32,	12
.equ	IRQ_INVALID_EL0_32,	13
.equ	FIQ_INVALID_EL0_32,	14
.equ	ERROR_INVALID_EL0_32,	15

.equ	STACK_OVERFLOW,		16

// Ventry aligns to 128 bytes and branches to label, and is used for the interrupt vector table
.macro	ventry	label
.balign	128
	b	\label
.endm

// TrapFrame layout.  Must match the TrapFrame struct in trap.rs.
TF_LR_ESR	= 16 * 15	// x30, ESR_EL1
TF_ELR_FAR	= 16 * 16	// ELR_EL1, FAR_EL1
TF_TYPE_SPEL0	= 16 * 17	// Interrupt type, SP_EL0
TF_SPSR		= 16 * 18	// SPSR_EL1
TF_SIZE		= 16 * 19

// We save all general purpose registers, and a few useful system registers:
// - x0-28
// - x29 (Frame pointer)
// - x30 (Link register)
// We additionally save the registers below, so that they're available via the TrapFrame
// - ESR_EL1 (Exception syndrome register EL1)
// - ELR_EL1 (Exception link register EL1)
// - FAR_EL1 (Fault address register EL1)
// - SP_EL0 (User stack pointer)
// - SPSR_EL1 (Saved program status register EL1)
//
// The frame is pushed onto the current stack (SP_EL1).  For exceptions taken
// from EL1 that's the interrupted kernel stack.  For exceptions taken from EL0
// it's whatever SP_EL1 was set to before the eret into EL0 (see to_user).
// No general purpose register can be used until it has been saved.
.macro save_frame type
	stp 	x0, x1, [sp, #16 * 0]
	stp 	x2, x3, [sp, #16 * 1]
	stp	x4, x5, [sp, #16 * 2]
	stp	x6, x7, [sp, #16 * 3]
	stp	x8, x9, [sp, #16 * 4]
	stp	x10, x11, [sp, #16 * 5]
	stp	x12, x13, [sp, #16 * 6]
	stp	x14, x15, [sp, #16 * 7]
	stp	x16, x17, [sp, #16 * 8]
	stp	x18, x19, [sp, #16 * 9]
	stp	x20, x21, [sp, #16 * 10]
	stp	x22, x23, [sp, #16 * 11]
	stp	x24, x25, [sp, #16 * 12]
	stp	x26, x27, [sp, #16 * 13]
	stp	x28, x29, [sp, #16 * 14]

	// LR, ESR_EL1
	mrs	x0, esr_el1
	stp	x30, x0, [sp, #TF_LR_ESR]

	// ELR_EL1, FAR_EL1
	mrs	x1, elr_el1
	mrs	x2, far_el1
	stp	x1, x2, [sp, #TF_ELR_FAR]

	// Interrupt type, SP_EL0
	ldr	x3, =\type
	mrs	x4, sp_el0
	stp	x3, x4, [sp, #TF_TYPE_SPEL0]

	// SPSR_EL1
	mrs	x5, spsr_el1
	str	x5, [sp, #TF_SPSR]
.endm

// Reserve a TrapFrame on the stack, and check sp is still inside its stack,
// as Linux arm64's kernel_ventry does.  Every EL1 stack is the upper half of
// a block aligned to twice its size, so a valid sp has bit KSTACK_SHIFT set,
// and one that has run into the guard below has it clear.  The test swaps sp
// and x0 arithmetically, so no register is lost.  The EL1t vectors don't
// check: there sp is SP_EL0, which r9 never uses at EL1.
.macro handle_interrupt type, checksp=1
	sub 	sp, sp, #TF_SIZE
	.if \checksp
	add	sp, sp, x0			// sp' = sp + x0
	sub	x0, sp, x0			// x0' = sp
	tbz	x0, #{KSTACK_SHIFT}, stack_overflow
	sub	x0, sp, x0			// x0'' = x0
	sub	sp, sp, x0			// sp'' = sp
	.endif

	save_frame \type

	// Pass pointer to TrapFrame (on stack) as the first arg
	mov	x0, sp
	bl	trap_unsafe

	b	trap_return
.endm

// A kernel stack has overflowed: x0 is the overflowed sp, less a TrapFrame,
// and sp is that plus the original x0.  Nothing returns to EL0 from here, so
// stash both in EL0's thread registers, then report from the overflow stack.
stack_overflow:
	msr	tpidr_el0, x0
	sub	x0, sp, x0
	msr	tpidrro_el0, x0
	ldr	x0, =overflowstacktop
	mov	sp, x0
	mrs	x0, tpidrro_el0
	sub	sp, sp, #TF_SIZE
	save_frame STACK_OVERFLOW
	mov	x0, sp
	mrs	x1, tpidr_el0
	bl	stack_overflow_unsafe

// Restore the TrapFrame at sp and return from the exception.  The return
// state (ELR_EL1, SPSR_EL1, SP_EL0) is taken from the frame rather than the
// live registers, so trap handlers (and to_user) control where we return to.
.globl trap_return
trap_return:
	ldr	x0, [sp, #TF_ELR_FAR]
	msr	elr_el1, x0
	ldr	x0, [sp, #TF_TYPE_SPEL0 + 8]
	msr	sp_el0, x0
	ldr	x0, [sp, #TF_SPSR]
	msr	spsr_el1, x0

	ldp	x0, x1, [sp, #16 * 0]
	ldp	x2, x3, [sp, #16 * 1]
	ldp	x4, x5, [sp, #16 * 2]
	ldp	x6, x7, [sp, #16 * 3]
	ldp	x8, x9, [sp, #16 * 4]
	ldp	x10, x11, [sp, #16 * 5]
	ldp	x12, x13, [sp, #16 * 6]
	ldp	x14, x15, [sp, #16 * 7]
	ldp	x16, x17, [sp, #16 * 8]
	ldp	x18, x19, [sp, #16 * 9]
	ldp	x20, x21, [sp, #16 * 10]
	ldp	x22, x23, [sp, #16 * 11]
	ldp	x24, x25, [sp, #16 * 12]
	ldp	x26, x27, [sp, #16 * 13]
	ldp	x28, x29, [sp, #16 * 14]
	ldr	x30, [sp, #TF_LR_ESR]

	add	sp, sp, #TF_SIZE

	eret

/// The exception vector table for exceptions taken to EL1.
/// Each entry is 16 instructions/128 bytes.
/// Ventry handles alignment of individual entries.
.balign	2048
.globl exception_vectors
exception_vectors:
	// Current EL with SP0
	ventry	sync_invalid_el1t			// Synchronous EL1t
	ventry	irq_invalid_el1t			// IRQ EL1t
	ventry	fiq_invalid_el1t			// FIQ EL1t
	ventry	error_invalid_el1t			// Error EL1t

	// Current EL with SPx
	ventry	sync_invalid_el1h			// Synchronous EL1h
	ventry	irq_invalid_el1h			// IRQ EL1h
	ventry	fiq_invalid_el1h			// FIQ EL1h
	ventry	error_invalid_el1h			// Error EL1h

	// Lower EL using AArch64
	ventry	sync_invalid_el0_64			// Synchronous 64-bit EL0
	ventry	irq_invalid_el0_64			// IRQ 64-bit EL0
	ventry	fiq_invalid_el0_64			// FIQ 64-bit EL0
	ventry	error_invalid_el0_64			// Error 64-bit EL0

	// Lower EL using AArch32
	ventry	sync_invalid_el0_32			// Synchronous 32-bit EL0
	ventry	irq_invalid_el0_32			// IRQ 32-bit EL0
	ventry	fiq_invalid_el0_32			// FIQ 32-bit EL0
	ventry	error_invalid_el0_32			// Error 32-bit EL0

sync_invalid_el1t:
	handle_interrupt  SYNC_INVALID_EL1t, 0

irq_invalid_el1t:
	handle_interrupt  IRQ_INVALID_EL1t, 0

fiq_invalid_el1t:
	handle_interrupt  FIQ_INVALID_EL1t, 0

error_invalid_el1t:
	handle_interrupt  ERROR_INVALID_EL1t, 0

sync_invalid_el1h:
	handle_interrupt  SYNC_INVALID_EL1h

irq_invalid_el1h:
	handle_interrupt  IRQ_INVALID_EL1h

fiq_invalid_el1h:
	handle_interrupt  FIQ_INVALID_EL1h

error_invalid_el1h:
	handle_interrupt  ERROR_INVALID_EL1h

sync_invalid_el0_64:
	handle_interrupt  SYNC_INVALID_EL0_64

irq_invalid_el0_64:
	handle_interrupt  IRQ_INVALID_EL0_64

fiq_invalid_el0_64:
	handle_interrupt  FIQ_INVALID_EL0_64

error_invalid_el0_64:
	handle_interrupt  ERROR_INVALID_EL0_64

sync_invalid_el0_32:
	handle_interrupt  SYNC_INVALID_EL0_32

irq_invalid_el0_32:
	handle_interrupt  IRQ_INVALID_EL0_32

fiq_invalid_el0_32:
	handle_interrupt  FIQ_INVALID_EL0_32

error_invalid_el0_32:
	handle_interrupt  ERROR_INVALID_EL0_32
