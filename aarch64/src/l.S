// Aarch64 entry (Raspberry Pi 3, 4 focussed)

STACKSZ = 4096*4

.equ	CURRENTEL_EL,		(1<<3) | (1<<2)

.equ	SCR_EL3_NS,		(1<<0)
.equ	SCR_EL3_SMD,		(1<<7)
.equ	SCR_EL3_HCE,		(1<<8)
.equ	SCR_EL3_RW,		(1<<10)

.equ	SPSR_EL3_M_EL2H,	(1<<3) | (1<<0)	// Exception level and SP: EL2H
.equ	SPSR_EL3_F,		(1<<6)		// FIQ
.equ	SPSR_EL3_I,		(1<<7)		// IRQ
.equ	SPSR_EL3_A,		(1<<8)		// SError
.equ	SPSR_EL3_D,		(1<<9)		// Debug exception

.equ	HCR_EL2_RW,		(1<<31)

.equ	SPSR_EL2_M_EL1H,	(1<<2) | (1<<0)	// Exception level and SP: EL1h
.equ	SPSR_EL2_F,		(1<<6)		// FIQ
.equ	SPSR_EL2_I,		(1<<7)		// IRQ
.equ	SPSR_EL2_A,		(1<<8)		// SError
.equ	SPSR_EL2_D,		(1<<9)		// Debug exception

.equ	CPACR_EL1_FPEN,		(1<<21) | (1<<20)	// Don't trap FPU instr at EL1,0

.equ	TCR_EL1_T0SZ,		(16<<0)		// 2^(64-N) size offset of region addressed by TTBR0_EL1: 2^(64-N)
.equ	TCR_EL1_TG0,		(0<<14)		// TTBR0_EL1 4KiB granule
.equ	TCR_EL1_T1SZ,		(16<<16)	// 2^(64-N) size offset of region addressed by TTBR1_EL1: 2^(64-N)
.equ	TCR_EL1_TG1,		(2<<30)		// TTBR1_EL1 4KiB granule
.equ	TCR_EL1_IPS,		(1<<1)		// 40bit physical addresses (qemu default)

.equ	SCTLR_EL1_M,		(1<<0)		// Enable MMU

// Preset memory attributes.  This register stores 8 8-bit presets that are
// referenced by index in the page table entries:
//  [0] 0x00 - Device (Non-gathering, non-reordering, no early write acknowledgement (most restrictive))
//  [1] 0xff - Normal
.equ	MAIR_EL1,		0xff00
.equ	PT_MAIR_DEVICE,		(0<<2)		// Use device memory attributes
.equ	PT_MAIR_NORMAL,		(1<<2)		// Use normal memory attributes

.equ	PT_PAGE,		3		// 4KiB granule
.equ	PT_BLOCK,		1		// 2MiB granule

// Page table entry AP Flag
.equ	PT_AP_KERNEL_RW,	(0<<6)		// Kernel: rw
.equ	PT_AP_KERNEL_RW_USER_RW,(1<<6)		// Kernel: rw, User: rw
.equ	PT_AP_KERNEL_RO,	(2<<6)		// Kernel: r
.equ	PT_AP_KERNEL_RO_USER_RO,(3<<6)		// Kernel: r, User: r

.equ	PT_AF,			(1<<10)		// Access Flag

.equ	PT_PXN,			(1<<53)		// Priviledged execute never
.equ	PT_UXN,			(1<<54)		// User execute never

// Cache shareability
.equ	PT_NOSH,		(0<<8)		// Non-shareable (single core)
.equ	PT_OSH,			(2<<8)		// Outer shareable (shared across CPUs, GPU)
.equ	PT_ISH,			(3<<8)		// Inner shareable (shared across CPUs)

// This defines the kernel's virtual address location.
// This value splits a 48 bit address space exactly in half, with the half
// beginning with 1 going to the kernel.
.equ	KZERO,			0xffff800000000000
.equ	MiB,			(1<<20)
.equ	GiB,			(1<<30)
.equ	KTZERO,			(KZERO+2*MiB)	// Virtual base of kernel text

.section .boottext, "awx"
.globl start
start:
	// Cache dtb pointer so we can pass to main9 later
	mov	x27, x0
	// Cache entrypoint (offset)
	mov	x28, x4

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

el1:	// In EL1

	// AArch64 memory management examples
	//  https://developer.arm.com/documentation/102416/0100

	// AArch64 Address Translation
	//  https://developer.arm.com/documentation/100940/0101

	// The kernel has been loaded at the entrypoint, but the
	// addresses used in the elf are virtual addresses in the higher half.
	// If we try to access them, the CPU will trap, so the next step is to
	// enable the MMU and identity map the kernel virtual addresses to the
	// physical addresses that the kernel was loaded into.

	// The Aarch64 is super flexible.  We can have page tables (granules)
	// of 4, 16, or 64KiB.  If we assume 4KiB granules, we would have:
	//  [47-39] Index into L4 table, used to get address of the L3 table
	//  [38-30] Index into L3 table, used to get address of the L2 table
	//  [29-21] Index into L2 table, used to get address of the L1 table
	//  [20-12] Index into L1 table, used to get address of physical page 
	//  [11-0]  Offset into physical page corresponding to virtual address
	// L4-L1 simply refers to the page table with L1 always being the last
	// to be translated, giving the address of the physical page.
	// With a 4KiB granule, each index is 9 bits, so there are 512 (2^9)
	// entries in each table.  In this example the physical page would
	// also be 4KiB.
	
	// If we reduce the number of page tables from 4 to 3 (L3 to L1),
	// we have 21 bits [20-0] for the physical page offset, giving 2MiB
	// pages.  If we reduce to 2 tables, we have 30 bits [29-0], giving
	// 1GiB pages.

	// If we use 16KiB granules, the virtual address is split as follows:
	//  [46-36] Index into L3 table, used to get address of the L2 table
	//  [35-25] Index into L2 table, used to get address of the L1 table
	//  [24-14] Index into L1 table, used to get address of physical page 
	//  [13-0]  Offset into physical page corresponding to virtual address
	// The 14 bits in the offset results in 16KiB pages.  Each table is
	// 16KiB, consisting of 2048 entries, so requiring 11 bits per index.
	// If we instead use only 2 levels, that gives us bits [24-0] for the
	// offset into the physical page, which gives us 32MiB page size.

	// Finally, if we use 64KiB granules, the virtual address is split as
	// follows:
	//  [41-29] Index into L2 table, used to get address of the L1 table
	//  [28-16] Index into L1 table, used to get address of physical page
	//  [15-0]  Offset into physical page corresponding to virtual address
	// The 16 bits in the offset results in 64KiB pages.  Each table is
	// 64KiB, consisting of 8192 entries, so requiring 13 bits per index.
	// If we instead use only 1 level, that gives us bits [28-0] for the
	// offset into the physical page, which gives us 512MiB page size.

	// The address of the top level table is stored in the translation table
	// base registers.  ttbr0_el1 stores the address for the user space,
	// ttbr1_el1 stores the address for the kernel, both for EL1.
	// By default, ttbr1_el1 is used when the virtual address bit 55 is 1
	// otherwise ttbr0_el1 is used.

	// Memory attributes are set per page table entry, and are hierarchical,
	// so settings at a higher page affect those they reference.

	// Set up root tables for lower (ttbr0_el1) and higher (ttbr1_el1)
	// addresses.  kernelpt4 is the root of the page hierarchy for addresses
	// of the form 0xffff800000000000 (KZERO and above), while physicalpt4
	// handles 0x0000000000000000 until KZERO.  Although what we really
	// want is to move to virtual higher half addresses, we need to have
	// ttbr0_el1 identity mapped during the transition until the PC is also
	// in the higher half.  This is because the PC is still in the lower
	// half immediately after the MMU is enabled.
	adrp	x0, kernelpt4
	msr	ttbr1_el1, x0
	adrp	x0, physicalpt4
	msr	ttbr0_el1, x0

	// Set up the translation control register tcr_el1 as so:
	//  TCR_EL1_T0SZ: Size offset of region addressed by TTBR0_EL1: 2^30)
	//  TCR_EL1_T1SZ: Size offset of region addressed by TTBR1_EL1: 2^30)
	//  TCR_EL1_TG0: 4KiB granule
	//  TCR_EL1_TG1: 4KiB granule
	//  TCR_EL1_IPS: 40 bit physical addresses
	ldr	x0, =(TCR_EL1_T0SZ|TCR_EL1_T1SZ|TCR_EL1_TG0|TCR_EL1_TG1|TCR_EL1_IPS)
	msr	tcr_el1, x0

	// The mair_el1 register contains 8 different cache settings, to be
	// referenced by index by any page table entry.
	ldr	x0, =(MAIR_EL1)
	msr	mair_el1, x0

	// Force changes to be be seen before MMU enabled, then enable MMU
	isb
	mrs	x0, sctlr_el1
	ldr	x1, =(SCTLR_EL1_M)
	orr	x0, x0, x1
	msr	sctlr_el1, x0

	// Force changes to be be seen by next instruction.
	// At this point the PC is still in the lower half, so we need to jump
	// up to the higher half.
	isb
	mrs	x0, elr_el1
	ldr	x20, =(higher_half)
	br	x20

higher_half:
	// Now that the kernel is mapped, the MMU is enabled and we're in the
	// higher half, we can set up the initial stack.
	ldr	x0, =stack
	add	x0, x0, #STACKSZ
	mov	sp, x0

	// Clear bss
	ldr	x0, =bss		// Start address
	ldr	x1, =end		// End of bss
1:	str	xzr, [x0], #8
	cmp	x0, x1
	b.ne	1b

	// Jump to rust, passing DTB pointer (in x27, then map to upper half)
	ldr	x0, =(KZERO)
	add	x0, x0, x27
	bl	main9

.globl dnr
dnr:	wfe
	b	dnr

// Early page tables for mapping the kernel to the higher half.
// It's assumed that the kernelpt* page tables will only be used until the
// full VM code is running.
.balign 4096
kernelpt4:
	.space	(4096/2)
	.quad	(kernelpt3 - KZERO) + (PT_PAGE)
	.space	(4096/2) - (1*8)

.balign 4096
kernelpt3:
 	.quad	(0*2*GiB) + (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_MAIR_NORMAL)
	.space	(4096) - (1*8)

// Early page table for identity mapping the kernel physical addresses.
// Once we've jumped to the higher half, this will no longer be used.
// (The pt3 table is the same for in both cases, so we can share.)
.balign 4096
physicalpt4:
	.quad	(kernelpt3 - KZERO) + (PT_PAGE)
	.space	(4096) - (1*8)

.bss
.balign	4096
stack:	.space STACKSZ
