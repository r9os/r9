// Aarch64 entry (Raspberry Pi 4 focused)
//
// main.rs passes the constants this uses as global_asm! operands, so this
// file is a format string: literal braces must be doubled.

.section .boottext, "awx"
.globl start
start:
	// x27 holds the DTB's physical address until it's passed to main9.
	// The Rust functions called before then preserve it (AAPCS64).
	mov	x27, x0

	// All cores other than 0 should just hang
	mrs	x0, mpidr_el1
	and	x0, x0, #0xff
	cbnz	x0, dnr

	// Aarch64 has 4 exception levels:
	//  EL0 - Application level
	//  EL1 - OS
	//  EL2 - Hypervisor
	//  EL3 - Firmware
	// We want to be in EL1.  QEMU's -kernel enters at EL2, and the Pi's
	// firmware is expected to.  r9 doesn't support entering at EL3, so a
	// core there parks.

	// Dispatch to code to handle the current exception level
	mrs	x0, CurrentEL
	ubfx	x0, x0, #2, #2
	cmp	x0, #1
	beq	el1
	cmp	x0, #2
	bne	dnr

	// In EL2, so prepare to drop to EL1
	ldr	x0, ={HCR_EL2}
	msr	hcr_el2, x0
	ldr	x0, ={SPSR_EL2}
	msr	spsr_el2, x0
	ldr	x0, ={CPACR_EL1}
	msr	cpacr_el1, x0

	// Return to EL1
	adr	x0, el1
	msr	elr_el2, x0
	eret

el1:	// In EL1

	// Set up the initial stack.  The MMU is off, so adrp gives its
	// physical address.
	adrp	x0, stacktop
	add	x0, x0, #:lo12:stacktop
	mov	sp, x0

	// Clear BSS, which includes the early pagetables
	ldr	x0, =bss_pa
	ldr	x1, =ebss_pa
1:	str	xzr, [x0], #8
	cmp	x0, x1
	b.ne	1b

	mov	x0, x27			// DTB pointer
	bl	pre_mmu_init

	// Force changes to be seen before MMU enabled, then enable MMU
	dsb sy
	mrs	x0, sctlr_el1
	ldr	x1, ={SCTLR_EL1_MMU}
	orr	x0, x0, x1
	msr	sctlr_el1, x0		// Enable MMU!

	// Force changes to be seen by next instruction.
	// At this point the PC is still in the lower half, so we need to jump
	// up to the higher half.
	isb
	ldr	x20, =(higher_half)
	br	x20

higher_half:
	// Now that the kernel is mapped, the MMU is enabled and we're in the
	// higher half, we can set up the initial stack using the virtual address.
	ldr	x0, =stacktop
	mov	sp, x0

	// Jump to rust, passing the DTB's physical address
	mov	x0, x27
	bl	main9

.globl dnr
dnr:	wfe
	b	dnr

.bss
.balign	4096
stack:	.space {STACKSZ}
stacktop:
