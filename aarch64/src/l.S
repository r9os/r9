// Aarch64 entry (Raspberry Pi 3, 4 focussed)
 
.section .boottext, "awx"
.globl start

start:
	// All cores other than 0 should just hang
	mrs     x0, mpidr_el1
	and     x0, x0, #0xff
	cbnz    x0, hang
	
	// On the main core, so set up the initial stack
	ldr	x0, =start
	add	x0, x0, #4096
	mov	sp, x0

	// Clear bss
	ldr     x0, =bss     		// Start address
	ldr     x1, =end  		// End of bss
clearbss:
  	str     xzr, [x0], #8
	cmp	x0, x1
	b.ne	clearbss

	bl	main9

hang:	wfe
	b	hang
