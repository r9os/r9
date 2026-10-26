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
	ldr	x0, ={CPTR_EL2}
	msr	cptr_el2, x0

	// Return to EL1
	adr	x0, el1
	msr	elr_el2, x0
	eret

el1:	// In EL1

	// Most of SCTLR_EL1 resets to UNKNOWN values, the MMU and cache
	// enables included (Arm ARM DDI 0487 M.d, D24.2.174), and firmware
	// may have changed it, so set all of it.
	ldr	x0, ={SCTLR_EL1_MMU_OFF}
	msr	sctlr_el1, x0
	isb

	// Everything written before the MMU is on is in the BSS: the boot
	// stack, the early pagetables and the zeroed statics.  Those writes go
	// straight to memory, but the caches' contents at reset are
	// IMPLEMENTATION DEFINED (Arm ARM DDI 0487 M.d, D7.5.6), so firmware
	// may have left lines over the BSS.  A dirty one could be evicted over
	// what we write, and once SCTLR_EL1.C is set, a clean one could hide
	// it (D7.5.5).  So invalidate the BSS before writing it, as Linux does
	// for its early page tables (head.S, primary_entry).  No lines can be
	// allocated again while nothing at this EL or above maps the BSS as
	// cacheable (D7.5.1).
	// CTR_EL0.DminLine, bits [19:16], is log2 of the smallest data cache
	// line in words.  The BSS is page aligned, so no line holds anything
	// else.
	mrs	x2, ctr_el0
	ubfx	x2, x2, #16, #4
	mov	x3, #4
	lsl	x2, x3, x2
	ldr	x0, =bss_pa
	ldr	x1, =ebss_pa
1:	dc	ivac, x0
	add	x0, x0, x2
	cmp	x0, x1
	b.lo	1b
	dsb	sy

	// Clear BSS, which includes the early pagetables and the boot stack
	ldr	x0, =bss_pa
1:	str	xzr, [x0], #8
	cmp	x0, x1
	b.ne	1b

	// Set up the initial stack.  The MMU is off, so adrp gives its
	// physical address.
	adrp	x0, stacktop
	add	x0, x0, #:lo12:stacktop
	mov	sp, x0

	mov	x0, x27			// DTB pointer
	bl	pre_mmu_init

	// Force changes to be seen before MMU enabled, then enable MMU
	dsb sy
	ldr	x0, ={SCTLR_EL1_MMU_ON}
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
