// Aarch64 entry (Raspberry Pi 3, 4 focussed)

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

TCR_EL1_T0SZ			= (16<<0)		// 2^(64-N) size offset of region addressed by TTBR0_EL1: 2^(64-N)
TCR_EL1_TG0			= (0<<14)		// TTBR0_EL1 4KiB granule
TCR_EL1_T1SZ			= (16<<16)		// 2^(64-N) size offset of region addressed by TTBR1_EL1: 2^(64-N)
TCR_EL1_TG1			= (2<<30)		// TTBR1_EL1 4KiB granule
TCR_EL1_IPS			= (1<<1)		// 40bit physical addresses (qemu default)

SCTLR_EL1_M			= (1<<0)		// Enable MMU

// Preset memory attributes.  This register stores 8 8-bit presets that are
// referenced by index in the page table entries:
//  [0] 0x00 - Device (Non-gathering, non-reordering, no early write acknowledgement (most restrictive))
//  [1] 0xff - Normal
MAIR_EL1			= 0xff00
PT_MAIR_DEVICE			= (0<<2)		// Use device memory attributes
PT_MAIR_NORMAL			= (1<<2)		// Use normal memory attributes

PT_PAGE				= 3			// 4KiB granule
PT_BLOCK			= 1			// 2MiB granule

// Page table entry AP Flag
PT_AP_KERNEL_RW			= (0<<6)		// Kernel: rw
PT_AP_KERNEL_RW_USER_RW		= (1<<6)		// Kernel: rw, User: rw
PT_AP_KERNEL_RO			= (2<<6)		// Kernel: r
PT_AP_KERNEL_RO_USER_RO		= (3<<6)		// Kernel: r, User: r

PT_AF				= (1<<10)		// Access Flag

PT_PXN				= (1<<53)		// Priviledged execute never
PT_UXN				= (1<<54)		// User execute never

// Cache shareability
PT_NOSH				= (0<<8)		// Non-shareable (single core)
PT_OSH				= (2<<8)		// Outer shareable (shared across CPUs, GPU)
PT_ISH				= (3<<8)		// Inner shareable (shared across CPUs)

// This defines the kernel's virtual address location.
// This value splits a 48 bit address space exactly in half, with the half
// beginning with 1 going to the kernel.
KZERO				= 0xffff800000000000
MiB				= (1<<20)
GiB				= (1<<30)
KTZERO				= (KZERO + 2*MiB)	// Virtual base of kernel text

// Constants for early uart setup
MMIO_BASE_RPI3			= 0x3f000000
MMIO_BASE_RPI4			= 0xfe000000

GPIO				= 0x00200000	// Offset from MMIO base
GPFSEL1				= GPIO + 0x04
GPPUD				= GPIO + 0x94
GPPUDCLK0			= GPIO + 0x98
GPIO_PUP_PDN_CNTRL_REG0		= GPIO + 0xe4

GPFSEL1_FSEL14_MASK		= 0xffff8fff	// Mask all but bits 12:14 (pin 14)
GPFSEL1_FSEL14_ALT5		= 0x00002000	// Pin 14, ALT5
GPFSEL1_FSEL15_MASK		= 0xfffc7fff	// Mask all but bits 15:17 (pin 15)
GPFSEL1_FSEL15_ALT5		= 0x00010000	// Pin 15, ALT5
GPIO_PUP_PDN_CNTRL_REG0_MASK_14	= 0xcfffffff	// Mask all but bits 28:29
GPIO_PUP_PDN_CNTRL_REG0_MASK_15	= 0x3fffffff	// Mask all but bits 30:31
GPPUD_ENABLE_14_15		= 0x0000c000	// Enable pins 14, 15

AUX				= 0x00215000	// Offset from MMIO base
AUX_ENABLES			= AUX + 0x04
AUX_MU				= AUX + 0x40
AUX_MU_IER			= AUX_MU + 0x04
AUX_MU_IIR			= AUX_MU + 0x08
AUX_MU_LCR			= AUX_MU + 0x0c
AUX_MU_MCR			= AUX_MU + 0x10
AUX_MU_LSR			= AUX_MU + 0x14
AUX_MU_CNTL			= AUX_MU + 0x20
AUX_MU_BAUD			= AUX_MU + 0x28

// Calculate the baudrate to be inserted into AUX_MU_BAUD
UART_CLOCK			= 500000000
UART_BAUDRATE			= 115200
UART_BAUDRATE_REG		= ((UART_CLOCK/(UART_BAUDRATE*8))-1)

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

// Simple spin macro
//  reg: Scratch register
//  numcycles: Number of cycles to spin
.macro spin reg, numcycles
	mov	\reg, \numcycles
1:	isb
	subs	\reg, \reg, #1
	cbnz	\reg, 1b
.endm

// Write a character to the miniuart
// Overwrites x0, x1, x2
// Assumes MMIO base is in x26
.macro putc aux_mu, aux_mu_lsr, chr
	movz	x1, #:abs_g0_nc:\aux_mu
	movk	x1, #:abs_g1:\aux_mu
	add	x0, x1, x26
	movz	x1, #:abs_g0_nc:\aux_mu_lsr
	movk	x1, #:abs_g1:\aux_mu_lsr
	add	x1, x1, x26
	// Wait for miniuart to become ready to transmit, then write a byte
1\@:	ldr	w2, [x1]
	tbz	w2, #5, 1\@b
	mov	w2, \chr
	str	w2, [x0]
.endm

// Write newline to the miniuart
// Overwrites x0, x1, x2
// Assumes MMIO base is in x26
.macro putnewline aux_mu, aux_mu_lsr
	putc	\aux_mu, \aux_mu_lsr, '\r'
	putc	\aux_mu, \aux_mu_lsr, '\n'
.endm

// Define the macro to write a u64 in hex value using putc
// Overwrites x0, x1, x2, x3, x4
// Assumes MMIO base is in x26
.macro putu64 aux_mu, aux_mu_lsr, Xval
	# putc	\aux_mu, \aux_mu_lsr, '0'
	# putc	\aux_mu, \aux_mu_lsr, 'x'
	mov	x3, #60			// Amount to shift by
1\@:	lsr	x4, \Xval, x3
	and	x4, x4, #0xf		// Mask off digit
	cmp	x4, #10
	bge	3\@f
	add	x4, x4, #'0'		// 0-9
	b	4\@f
3\@:	sub	x4, x4, #10		// a-f
	add	x4, x4, #'a'
4\@:	putc	\aux_mu, \aux_mu_lsr, w4
	subs	x3, x3, #4
	bpl	1\@b
.endm

.section .boottext, "awx"
.globl start
start:
	// We use some registers throught this assembly code.  They shouldn't be
	// used by any code in this file.  Once we call main9, they can be
	// used again.  There's also a couple that are best avoided out of
	// principle.

	// x26: MMIO base (to be set later)
	// x27: DTB address
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

	// Set up a very early uart - the miniuart.  The full driver is in
	// uartmini.rs.  This code is just enough to help debug the early stage.
	// Unfortunately raspberry pi 3 and 4 have differences in miniuart init.

	// We can get the board ID from the midr_el1 register (PartNum [15:4]),
	// and use that to work out the appropriate MMIO base.
	mrs	x0, midr_el1
	lsr	x0, x0, #4
	and	x0, x0, #0xfff
	cmp	x0, #0xd03	// Check for rpi3
	bne	.rpi4uart

	// We must be on a raspberry pi 3, which has a different MMIO base, and
	// a different method for configuring the miniuart...
	mov	x26, MMIO_BASE_RPI3

	// x0 AUX_ENABLES
	movz	x1, #:abs_g0_nc:AUX_ENABLES
	movk	x1, #:abs_g1:AUX_ENABLES
	add	x0, x1, x26
	ldr	w1, [x0]
	orr	w1, w1, #1
	str	w1, [x0]

	// x0 AUX_MU_CNTL
	movz	x1, #:abs_g0_nc:AUX_MU_CNTL
	movk	x1, #:abs_g1:AUX_MU_CNTL
	add	x0, x1, x26
	str	wzr, [x0]

	// x0 AUX_MU_LCR
	movz	x1, #:abs_g0_nc:AUX_MU_LCR
	movk	x1, #:abs_g1:AUX_MU_LCR
	add	x0, x1, x26
	mov	w1, #3
	str	w1, [x0]

	// x0 AUX_MU_MCR
	movz	x1, #:abs_g0_nc:AUX_MU_MCR
	movk	x1, #:abs_g1:AUX_MU_MCR
	add	x0, x1, x26
	str	wzr, [x0]

	// x0 AUX_MU_IER
	movz	x1, #:abs_g0_nc:AUX_MU_IER
	movk	x1, #:abs_g1:AUX_MU_IER
	add	x0, x1, x26
	str	wzr, [x0]

	// x0 AUX_MU_IIR
	movz	x1, #:abs_g0_nc:AUX_MU_IIR
	movk	x1, #:abs_g1:AUX_MU_IIR
	add	x0, x1, x26
	mov	w1, #0xc6
	str	w1, [x0]

	// x0 AUX_MU_BAUD
	movz	x1, #:abs_g0_nc:AUX_MU_BAUD
	movk	x1, #:abs_g1:AUX_MU_BAUD
	add	x0, x1, x26
	mov	w1, UART_BAUDRATE_REG
	str	w1, [x0]

	// Set GPIO pins 14 to be used for ALT5 - UART1 (miniuart)
	// x0 GPFSEL1
	movz	x1, #:abs_g0_nc:GPFSEL1
	movk	x1, #:abs_g1:GPFSEL1
	add	x0, x1, x26
	ldr	w1, [x0]
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL14_MASK
	movk	w2, #:abs_g1:GPFSEL1_FSEL14_MASK
	and	w1, w1, w2
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL14_ALT5
	movk	w2, #:abs_g1:GPFSEL1_FSEL14_ALT5
	orr	w1, w1, w2
	str	w1, [x0]

	// Set GPIO pins 15 to be used for ALT5 - UART1 (miniuart)
	// x0 GPFSEL1
	movz	x1, #:abs_g0_nc:GPFSEL1
	movk	x1, #:abs_g1:GPFSEL1
	add	x0, x1, x26
	ldr	w1, [x0]
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL15_MASK
	movk	w2, #:abs_g1:GPFSEL1_FSEL15_MASK
	and	w1, w1, w2
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL15_ALT5
	movk	w2, #:abs_g1:GPFSEL1_FSEL15_ALT5
	orr	w1, w1, w2
	str	w1, [x0]

	// Set up GPIO pull up/down state
	// x0 GPPUD
	movz	x1, #:abs_g0_nc:GPPUD
	movk	x1, #:abs_g1:GPPUD
	add	x0, x1, x26
	str	wzr, [x0]

	spin	x3, #150

	// x0 GPPUDCLK0
	movz	x1, #:abs_g0_nc:GPPUDCLK0
	movk	x1, #:abs_g1:GPPUDCLK0
	add	x0, x1, x26
	ldr	w1, =GPPUD_ENABLE_14_15
	str	w1, [x0]
	
	spin	x3, #150

	// Write 0 to GPPUDCLK0
	str	wzr, [x1]

	// x0 AUX_MU_CNTL
	movz	x1, #:abs_g0_nc:AUX_MU_CNTL
	movk	x1, #:abs_g1:AUX_MU_CNTL
	add	x0, x1, x26
	mov	w1, #3
	str	w1, [x0]

	b	.uartinitdone

.rpi4uart:
	// We're on a raspberry pi 4, or assume future boards have the same uart
	mov	x26, MMIO_BASE_RPI4

	// x0 AUX_ENABLES
	movz	x1, #:abs_g0_nc:AUX_ENABLES
	movk	x1, #:abs_g1:AUX_ENABLES
	add	x0, x1, x26
	ldr	w1, [x0]
	orr	w1, w1, #1
	str	w1, [x0]

	// x0 AUX_MU_CNTL
	movz	x1, #:abs_g0_nc:AUX_MU_CNTL
	movk	x1, #:abs_g1:AUX_MU_CNTL
	add	x0, x1, x26
	str	wzr, [x0]

	// x0 AUX_MU_LCR
	movz	x1, #:abs_g0_nc:AUX_MU_LCR
	movk	x1, #:abs_g1:AUX_MU_LCR
	add	x0, x1, x26
	mov	w1, #3
	str	w1, [x0]

	// x0 AUX_MU_MCR
	movz	x1, #:abs_g0_nc:AUX_MU_MCR
	movk	x1, #:abs_g1:AUX_MU_MCR
	add	x0, x1, x26
	str	wzr, [x0]

	// x0 AUX_MU_IER
	movz	x1, #:abs_g0_nc:AUX_MU_IER
	movk	x1, #:abs_g1:AUX_MU_IER
	add	x0, x1, x26
	str	wzr, [x0]

	// x0 AUX_MU_IIR
	movz	x1, #:abs_g0_nc:AUX_MU_IIR
	movk	x1, #:abs_g1:AUX_MU_IIR
	add	x0, x1, x26
	mov	w1, #0xc6
	str	w1, [x0]

	// x0 AUX_MU_BAUD
	movz	x1, #:abs_g0_nc:AUX_MU_BAUD
	movk	x1, #:abs_g1:AUX_MU_BAUD
	add	x0, x1, x26
	mov	w1, UART_BAUDRATE_REG
	str	w1, [x0]

	// Set up GPIO pin 14 pull up/down state
	// x0 GPIO_PUP_PDN_CNTRL_REG0
	movz	x1, #:abs_g0_nc:GPIO_PUP_PDN_CNTRL_REG0
	movk	x1, #:abs_g1:GPIO_PUP_PDN_CNTRL_REG0
	add	x0, x1, x26
	ldr	w1, [x0]
	movz	w2, #:abs_g0_nc:GPIO_PUP_PDN_CNTRL_REG0_MASK_14
	movk	w2, #:abs_g1:GPIO_PUP_PDN_CNTRL_REG0_MASK_14
	and	w1, w1, w2
	// Pull none (0) - just use mask
	str	w1, [x0]

	// Set GPIO pins 14 to be used for ALT5 - UART1 (miniuart)
	// x0 GPFSEL1
	movz	x1, #:abs_g0_nc:GPFSEL1
	movk	x1, #:abs_g1:GPFSEL1
	add	x0, x1, x26
	ldr	w1, [x0]
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL14_MASK
	movk	w2, #:abs_g1:GPFSEL1_FSEL14_MASK
	and	w1, w1, w2
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL14_ALT5
	movk	w2, #:abs_g1:GPFSEL1_FSEL14_ALT5
	orr	w1, w1, w2
	str	w1, [x0]

	// Set up GPIO pin 15 pull up/down state
	// x0 GPIO_PUP_PDN_CNTRL_REG0
	movz	x1, #:abs_g0_nc:GPIO_PUP_PDN_CNTRL_REG0
	movk	x1, #:abs_g1:GPIO_PUP_PDN_CNTRL_REG0
	add	x0, x1, x26
	ldr	w1, [x0]
	movz	w2, #:abs_g0_nc:GPIO_PUP_PDN_CNTRL_REG0_MASK_15
	movk	w2, #:abs_g1:GPIO_PUP_PDN_CNTRL_REG0_MASK_15
	and	w1, w1, w2
	// Pull none (0) - just use mask
	str	w1, [x0]

	// Set GPIO pins 15 to be used for ALT5 - UART1 (miniuart)
	// x0 GPFSEL1
	movz	x1, #:abs_g0_nc:GPFSEL1
	movk	x1, #:abs_g1:GPFSEL1
	add	x0, x1, x26
	ldr	w1, [x0]
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL15_MASK
	movk	w2, #:abs_g1:GPFSEL1_FSEL15_MASK
	and	w1, w1, w2
	movz	w2, #:abs_g0_nc:GPFSEL1_FSEL15_ALT5
	movk	w2, #:abs_g1:GPFSEL1_FSEL15_ALT5
	orr	w1, w1, w2
	str	w1, [x0]

	// x0 AUX_MU_CNTL
	movz	x1, #:abs_g0_nc:AUX_MU_CNTL
	movk	x1, #:abs_g1:AUX_MU_CNTL
	add	x0, x1, x26
	mov	w1, #3
	str	w1, [x0]

.uartinitdone:
	putc	AUX_MU, AUX_MU_LSR, #'.'

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
	// half immediately after the MMU is enabled.  Once we enter rust-land,
	// we can define a new set of tables.
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
	msr	sctlr_el1, x0		// Enable MMU!

	// Force changes to be be seen by next instruction.
	// At this point the PC is still in the lower half, so we need to jump
	// up to the higher half.
	isb
	mrs	x0, elr_el1
	ldr	x20, =(higher_half)
	br	x20

higher_half:
	ldr	x0, =(KZERO)
	# ldr	x1, =(MMIO_BASE_RPI4 + GPIO)
	# add	x25, x1, x0
	add	x26, x26, x0
	# putnewline	AUX_MU, AUX_MU_LSR
	# putu64	AUX_MU, AUX_MU_LSR, x25
	# putnewline	AUX_MU, AUX_MU_LSR
	# at	s1e1r, x25
	# mrs	x25, par_el1
	# putu64	AUX_MU, AUX_MU_LSR, x25
	putc	AUX_MU, AUX_MU_LSR, #'m'
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

.macro	ventry	label
.balign	128
	b	\label
.endm

.macro handle_interrupt type
	putnewline	AUX_MU, AUX_MU_LSR

	// Interrupt type
	putc	AUX_MU, AUX_MU_LSR, #'0'
	putc	AUX_MU, AUX_MU_LSR, #'x'
	mov	x25, \type
	putu64	AUX_MU, AUX_MU_LSR, x25
	putnewline	AUX_MU, AUX_MU_LSR

	// esr_el1
	putc	AUX_MU, AUX_MU_LSR, #'0'
	putc	AUX_MU, AUX_MU_LSR, #'x'
	mrs	x25, esr_el1
	putu64	AUX_MU, AUX_MU_LSR, x25
	putnewline	AUX_MU, AUX_MU_LSR

	// elr_el1
	putc	AUX_MU, AUX_MU_LSR, #'0'
	putc	AUX_MU, AUX_MU_LSR, #'x'
	mrs	x25, elr_el1
	putu64	AUX_MU, AUX_MU_LSR, x25
	putnewline	AUX_MU, AUX_MU_LSR
	
1\@:	wfe
	b	1\@b
.endm

.balign	2048
.exception_vectors:
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
	handle_interrupt  SYNC_INVALID_EL1t

irq_invalid_el1t:
	handle_interrupt  IRQ_INVALID_EL1t

fiq_invalid_el1t:
	handle_interrupt  FIQ_INVALID_EL1t

error_invalid_el1t:
	handle_interrupt  ERROR_INVALID_EL1t

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

// Early page tables for mapping the kernel to the higher half.
// It's assumed that the kernelpt* page tables will only be used until the
// full VM code is running.

// Here we've set up a 2GiB page from the start of the kernel address space.
// This covers 0xffff_8000_0000_0000 - 0xffff_8000_8000_0000, and should be more
// than enough at this stage.

// We also want to map the MMIO section, which for the part of MMIO that we care
// about for Raspberry Pi 4 (to allow us to use the miniuart), is basically a
// 2MiB section starting at from 0xfe20_0000.  This is all in the lower half,
// so to allow us to abandon the physicalpt4 temp page table quickly, we'll
// map it into the higher half, starting at 0xffff_8000_fe20_0000.  Note that
// this is temporary - once we have a rust VM, the MMIO will be mapped somewhere
// else.

// Unfortunately, this is very specific to Raspberry Pi 4.  Once we're confident
// that the aarch64 setup code in l.S is solid, we should disable the uart code
// and perhaps have something that can be enabled manually for dev purposes only
// in the future.
.balign 4096
kernelpt4:
	.space	(256*8)
	.quad	(kernelpt3 - KZERO) + (PT_PAGE)		// [256] (for kernel + mmio)
	.space	(255*8)

.balign 4096
kernelpt3:
 	.quad	(0*2*GiB) + (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_MAIR_NORMAL)	// [0] (for kernel)
	.space	(2*8)
	.quad	(kernelpt2 - KZERO) + (PT_PAGE)		// [3] (for mmio)
	.space	(508*8)

.balign 4096
kernelpt2:
	.space	(497*8)
 	.quad	(MMIO_BASE_RPI4 + GPIO) + (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_MAIR_DEVICE)	// [497] (for mmio)
	.space	(14*8)

// Early page tables for identity mapping the kernel physical addresses.
// Once we've jumped to the higher half, this will no longer be used.
.balign 4096
physicalpt4:
	.quad	(physicalpt3 - KZERO) + (PT_PAGE)	// [0] (for kernel)
	.space	(511*8)

.balign 4096
physicalpt3:
 	.quad	(0*2*GiB) + (PT_BLOCK|PT_AF|PT_AP_KERNEL_RW|PT_ISH|PT_UXN|PT_MAIR_NORMAL)	// [0] (for kernel)
	.space	(511*8)

.bss
.balign	4096
stack:	.space STACKSZ
