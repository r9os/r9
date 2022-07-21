// Aarch64 entry (Raspberry Pi 3, 4 focussed)

STACKSZ = 4096*4

.equ	CURRENTEL_EL,		(1<<3) | (1<<2)

.equ	SCR_EL3_NS,		(1<<0)
.equ	SCR_EL3_SMD,		(1<<7)
.equ	SCR_EL3_HCE,		(1<<8)
.equ	SCR_EL3_RW,		(1<<10)

.equ	SPSR_EL3_M_EL2H,	(1<<3) | (1<<0)	// Exception level and SP: EL2H
.equ	SPSR_EL3_F,		(1<<6)	// FIQ
.equ	SPSR_EL3_I,		(1<<7)	// IRQ
.equ	SPSR_EL3_A,		(1<<8)	// SError
.equ	SPSR_EL3_D,		(1<<9)	// Debug exception

.equ	HCR_EL2_RW,		(1<<31)
.equ	HCR_EL2_SWIO,		(1<<1)

.equ	SPSR_EL2_M_EL1H,	(1<<2) | (1<<0)	// Exception level and SP: EL1h
.equ	SPSR_EL2_F,		(1<<6)	// FIQ
.equ	SPSR_EL2_I,		(1<<7)	// IRQ
.equ	SPSR_EL2_A,		(1<<8)	// SError
.equ	SPSR_EL2_D,		(1<<9)	// Debug exception

.equ	CPACR_EL1_FPEN,		(1<<21) | (1<<20)	// Don't trap FPU instr at EL1,0

.section .boottext, "awx"
.globl start
start:
	// All cores other than 0 should just hang
	mrs	x0, mpidr_el1
	and	x0, x0, #0xff
	cbnz	x0, dnr

	// Aarch64 has 4 exception levels:
	//  EL0 - Application level
	//  EL1 - Rich OS
	//  EL2 - Hypervisor
	//  EL3 - Firmware
	// We want to be in EL1.  Qemu starts in EL3.  Raspi3 usually starts in EL2.

	// Dispatch to code to handle the current exception level
	mrs	x0, CurrentEL
	and	x0, x0, CURRENTEL_EL
	lsr	x0, x0, #2
	cmp	x0, #1
	beq	el1
	cmp	x0, #2
	beq	el2

	// Must be EL3, so prepare jump to EL2
	ldr	x0, =(SCR_EL3_NS|SCR_EL3_SMD|SCR_EL3_HCE|SCR_EL3_RW)
	msr	scr_el3, x0
	ldr	x0, =(SPSR_EL3_M_EL2H|SPSR_EL3_F|SPSR_EL3_I|SPSR_EL3_A|SPSR_EL3_D)
	msr	spsr_el3, x0

	// Return to EL2
	adr	x0, el2
	msr	elr_el3, x0
	eret
	
el2:	// Now in EL2, so prepare jump to EL1
	// Enable AArch64 in EL1
	ldr	x0, =HCR_EL2_RW
	msr	hcr_el2, x0
	ldr	x0, =(SPSR_EL2_M_EL1H|SPSR_EL2_F|SPSR_EL2_I|SPSR_EL2_A|SPSR_EL2_D)
	msr	spsr_el2, x0

	// Enable FPU in EL1, EL0
	ldr	x0, =CPACR_EL1_FPEN
	msr	cpacr_el1, x0

	// Return to EL1
	adr	x0, el1
	msr	elr_el2, x0
	eret

el1:	// In EL1, so set up the initial stack
	ldr	x0, =stack
	add	x0, x0, #STACKSZ
	mov	sp, x0

	// Clear bss
	ldr	x0, =bss		// Start address
	ldr	x1, =end		// End of bss
1:
	str	xzr, [x0], #8
	cmp	x0, x1
	b.ne	1b

	// Finally jump to rust
	bl	main9

.globl dnr
dnr:	wfe
	b	dnr

.bss
.balign	4096
stack:	.space STACKSZ
