// Aarch64 entry (Raspberry Pi 3, 4 focussed)

STACKSZ = 4096*4

.section .boottext, "awx"
.globl start
start:
	// All cores other than 0 should just hang
	mrs	x0, mpidr_el1
	and	x0, x0, #0xff
	cbnz	x0, dnr
	
	// On the main core, so set up the initial stack
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

	bl	main9

.globl dnr
dnr:	wfe
	b	dnr

.bss
.balign	4096
stack:	.space STACKSZ
