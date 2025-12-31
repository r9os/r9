// Aarch64 entry (Raspberry Pi 4 focused)

STACKSZ = 4096*4

CURRENTEL_EL			= (1<<3) | (1<<2)

SCR_EL3_NS			= (1<<0)
SCR_EL3_SMD			= (1<<7)
SCR_EL3_HCE			= (1<<8)
SCR_EL3_RW			= (1<<10)

SPSR_EL3_M_EL2H			= (1<<3) | (1<<0)	// Exception level and SP: EL2H
SPSR_EL3_F			= (1<<6)		// FIQ
SPSR_EL3_I			= (1<<7)		// IRQ
SPSR_EL3_A			= (1<<8)		// SError
SPSR_EL3_D			= (1<<9)		// Debug exception

HCR_EL2_RW			= (1<<31)

SPSR_EL2_M_EL1H			= (1<<2) | (1<<0)	// Exception level and SP: EL1h
SPSR_EL2_F			= (1<<6)		// FIQ
SPSR_EL2_I			= (1<<7)		// IRQ
SPSR_EL2_A			= (1<<8)		// SError
SPSR_EL2_D			= (1<<9)		// Debug exception

CPACR_EL1_FPEN			= (1<<21) | (1<<20)	// Don't trap FPU instr at EL1,0

SCTLR_EL1_I			= (1 << 12)		// Instruction access cacheability
SCTLR_EL1_C			= (1 << 2)		// Data cacheability
SCTLR_EL1_M			= (1 << 0)		// Enable MMU
SCTLR_EL1			= (SCTLR_EL1_I|SCTLR_EL1_C|SCTLR_EL1_M)

// This defines the kernel's virtual address location.
// This value splits a 48 bit address space exactly in half, with the half
// beginning with 1 going to the kernel.
KZERO				= 0xffff800000000000

// Exception vector IDs
SYNC_INVALID_EL1t		= 0
IRQ_INVALID_EL1t		= 1
FIQ_INVALID_EL1t		= 2
ERROR_INVALID_EL1t		= 3

SYNC_INVALID_EL1h		= 4
IRQ_INVALID_EL1h		= 5
FIQ_INVALID_EL1h		= 6
ERROR_INVALID_EL1h		= 7

SYNC_INVALID_EL0_64		= 8
IRQ_INVALID_EL0_64		= 9
FIQ_INVALID_EL0_64		= 10
ERROR_INVALID_EL0_64		= 11

SYNC_INVALID_EL0_32		= 12
IRQ_INVALID_EL0_32		= 13
FIQ_INVALID_EL0_32		= 14
ERROR_INVALID_EL0_32		= 15

.section .boottext, "awx"
.globl start
start:
	// We use some registers throught this assembly code.  They shouldn't be
	// used by any code in this file.  Once we call main9, they can be
	// used again.  There's also a couple that are best avoided out of
	// principle.

	// x27: DTB address (physical address)
	// x28: Entrypoint address
	// x29: Frame pointer
	// x30: Link register
	mov	x27, x0			// Cache dtb pointer so we can pass to main9 later
	mov	x28, x4			// Cache entrypoint (offset)

	// All cores other than 0 should just hang
	mrs	x0, mpidr_el1
	and	x0, x0, #0xff
	cbnz	x0, dnr

	// Aarch64 has 4 exception levels:
	//  EL0 - Application level
	//  EL1 - OS
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

el1:	// In EL1

	// Set up the initial stack using the physical address
	ldr	x0, =stack
	ldr	x2, =(KZERO)
	sub	x0, x0, x2
	add	x0, x0, #STACKSZ
	mov	sp, x0

	// Clear BSS using physical addresses
	ldr	x0, =bss		// Start address
	sub	x0, x0, x2
	ldr	x1, =end		// End of bss
	sub	x1, x1, x2
1:	str	xzr, [x0], #8
	cmp	x0, x1
	b.ne	1b

	bl	init_early_uart_rpi4

	mov	x0, #'.'
	bl	init_early_uart_putc

	mov	x0, x27			// DTB pointer
	bl	init_vm

	// Force changes to be be seen before MMU enabled, then enable MMU
	isb
	ldr	x0, =(SCTLR_EL1)
	msr	sctlr_el1, x0		// Enable MMU!

	// Force changes to be be seen by next instruction.
	// At this point the PC is still in the lower half, so we need to jump
	// up to the higher half.
	isb
	mrs	x0, elr_el1
	ldr	x20, =(higher_half)
	br	x20

higher_half:
	// Now that the kernel is mapped, the MMU is enabled and we're in the
	// higher half, we can set up the initial stack using the virtual address.
	ldr	x0, =stack
	add	x0, x0, #STACKSZ
	mov	sp, x0

	// Jump to rust, passing DTB pointer (in x27, then map to upper half)
	ldr	x0, =(KZERO)
	add	x0, x0, x27
	bl	main9

.globl dnr
dnr:	wfe
	b	dnr

.bss
.balign	4096
stack:	.space STACKSZ
