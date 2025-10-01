// Position independent code for early boot.
//
// It gets ugly to try to link this at some low address
// and then have the rest of the kernel linked high; that
// goes doubly for any attempt to load at a random address.

// Useful definitions.
.set GdtNULL,			(0<<3)
.set GdtCODE64,			(1<<3)
.set GdtCODE32,			(2<<3)
.set GdtDATA32,			(3<<3)

.set SegREAD,			(1<<41)
.set SegWRITE,			(1<<42)
.set SegCODE,			(1<<43)
.set SegDATA,			(0<<43)
.set SegMB1,			(1<<44)
.set SegPRESENT,		(1<<47)
.set SegLONG,			(1<<53)

.set Seg32DEFAULT,		(1<<54)
.set Seg32GRAN,			(1<<55)
.set Seg32LIMIT,		((0xF<<48)+0xFFFF)
.set Seg32DEF,			(Seg32DEFAULT|Seg32GRAN|Seg32LIMIT)

.set MULTIBOOT_FLAG_PGALIGN,	(1<<0)
.set MULTIBOOT_FLAG_MEMINFO,	(1<<1)
.set MULTIBOOT_MAGIC,		0x1BADB002
.set MULTIBOOT_FLAGS,		(MULTIBOOT_FLAG_PGALIGN | MULTIBOOT_FLAG_MEMINFO)
.set MULTIBOOT_CHECKSUM,	-(MULTIBOOT_MAGIC + MULTIBOOT_FLAGS)

.set KiB,			(1<<10)
.set MiB,			(1<<20)
.set GiB,			(1<<30)

.set PGSZ,			(4*KiB)

.set KZERO,			0xffffffff80000000
.set KCPUZERO,			0xffffff0000000000
.set KMACH,			(1*MiB)
.set KTOFF,			(2*MiB)
.set KLOSTK,			0x8000

.set MACHSTKSZ,			(64*KiB)
.set MACHSTKOFF,		(64*KiB)
.set MACHGDTSZ,			(64*KiB)
.set MACHGDTOFF,		(192*KiB)

.set Cr0PE,			(1<<0)		// Protected Mode Enable
.set Cr0MP,			(1<<1)		// Monitor Coprocessor
.set Cr0TS,			(1<<7)		// Task Switched
.set Cr0WP,			(1<<16)		// Write Protect
.set Cr0NW,			(1<<29)		// Not Writethrough
.set Cr0CD,			(1<<30)		// Cache Disable
.set Cr0PG,			(1<<31)		// Paging Enable

.set Cr4PSE,			(1<<4)		// Page-Size Extensions
.set Cr4PAE,			(1<<5)		// Physical Address Extension
.set Cr4PGE,			(1<<7)		// Page-Global Enable
.set Cr4FSGSBASE,		(1<<16)		// (RD|WR)(FS|GS)BASE Extension

.set IA32_EFER,			0xc0000080	// Extended Feature Enable

.set EferSCE,			(1<<0)		// System Call Extension
.set EferLME,			(1<<8)		// Long Mode Enable
.set EferNXE,			(1<<11)		// No-Execute Enable

.set PteP,			(1<<0)		// Present
.set PteRW,			(1<<1)		// Read/Write
.set PtePS,			(1<<7)		// Page Size
.set PteHiNX,			(1<<31)		// NX in the bit 4 bytes of PTE

.set I8259A0,			0x20
.set I8259A1,			0xA0

// When we get here we are in protected mode with a GDT.  We set
// up IA32e mode and get into long mode with paging enabled, then
// jump to Rust.
//
// This code is thus responsible for setting up the parts of the
// Mach required to get us into Rust code, running against our linked
// addresses.  This means setting up the parts of the Mach required
// for initial entry into Rust code, and in particular hand-crafting
// the initial page tables for early boot (the space for which are
// in the Mach).
//
// The initial page tables provide an (nearly) identity mapping for the first
// 0-4GiB of physical address space to
// KZERO, in addition to an identity map
// for the switch from protected to paged mode.  There
// is an assumption here that the creation and later
// removal of the identity map will not interfere with
// the KZERO mappings.
//
// The bulk of Mach[0] initialization, including remapping the
// kernel and setting up most of the architecturally required
// state, happens in Rust code.  We just do page tables and set
// up the stack here.
//
// Note that this code relies on details of the layout of the
// Mach.
.balign 4096
.section .text.boot, "ax"

.code32
.globl start
start:
	jmp	1f	// Jump over the multiboot header
	// Multiboot 1 header.
	.balign 8
	.long	MULTIBOOT_MAGIC
	.long	MULTIBOOT_FLAGS
	.long	MULTIBOOT_CHECKSUM
	.balign 16
1:
	cli
	cld

	// Save the multiboot information pointer.
	movl	%ebx, %ebp

	// Mask off the PIC.  We never use it.
	movw	$(I8259A0 + 1), %dx
	movb	$0xFF, %al
	outb	%al, %dx
	movw	$(I8259A1 + 1), %dx
	outb	%al, %dx

	// Zero the second mibibyte of RAM.  This region holds
	// the Mach for core 0, which in turn holds space for the
	// various architecturally defined state required for
	// handling traps and syscalls, switching processes, and
	// so on.
	//
	// We could also zero the BSS here, but the loader does it
	// for us.
	movl	$KMACH, %edi
	movl	$(KTOFF-KMACH), %ecx
	xorl	%eax, %eax
	rep stosb

	// Load the physical address of the Mach into the base register
	movl	$KMACH, %ebx

	// Load the stack pointer
	leal	MACHSTKOFF+MACHSTKSZ(%ebx), %esp

	// The PML4 we ordinarily run on lives inside of the Mach.
	// However, we'd much rather set it up in Rust code, not here,
	// so we use a set of simple, hard-coded page tables early on
	// in boot.  These place the Mach at the expected location, so
	// that we can refer to our stack at the correct virtual address
	// space, but do not handle anything beyond the bare minimum.
	movl	$(bootpml4 - KZERO), %eax
	movl	%eax, %cr3

	// Enable global pages and the physical address extension.  Turn
	// off the page size extension; it is always eanbled in PAE mode.
	movl	%cr4, %eax
	andl	$~Cr4PSE, %eax
	orl	$(Cr4PGE|Cr4PAE|Cr4FSGSBASE), %eax
	movl	%eax, %cr4

	// Set up the Extended Feature Enable (EFER) MSR, enabling
	// long mode, the NX bit in PTEs, and the SYSCALL/SYSRET
	// instructions.
	movl	$IA32_EFER, %ecx	// Extended Feature Enable
	rdmsr
	orl	$(EferSCE|EferLME|EferNXE), %eax
	wrmsr

	// Enable caching, paging, and page permisson enforcement in
	// kernel mode.
	movl	%cr0, %eax
	andl	$~(Cr0CD|Cr0NW|Cr0TS|Cr0MP), %eax
	orl	$(Cr0PG|Cr0WP), %eax
	movl	%eax, %cr0

	// Load the 64-bit GDT
	movl	$(gdtdesc-KZERO), %eax
	lgdt	(%eax)

	// Jump into 64 bit code.
	ljmpl	$GdtCODE64, $(1f-KZERO)

.code64
1:
	// Long mode.  Welcome to 2003.  Reload the GDT at its linked
	// linear address, and jump out of the identity map and into
	// the kernel address space.

	// Load a 64-bit GDT in the kernel address space.
	movabsq	$gdtdescv, %rax
	lgdt	(%rax)
	pushq	$GdtCODE64
	pushq	$(1f-KZERO)
	lretq
1:
	// Zero out the segment registers: they are not used in long mode.
	xorl	%eax, %eax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %ss
	movw	%ax, %fs
	movw	%ax, %gs

	// We can now use linked addresses for the stack and code.
	// Jump to the code that initialized ccNUMA node 0 and the
	// cpu0 Mach, returning to warp64, which removes the
	// identity map, sets the SP to the scheduler stack, and
	// jumps
	leaq	KMACH, %rdi
	leaq	KLOSTK, %rsp
	movabsq	$warp64, %rax
	pushq	%rax
	movabsq	$init0, %rax
	pushq	%rax
	ret

warp64:
	// Now load the stack pointer with our Mach stack.
	movabsq	$(KCPUZERO+KMACH), %rdi
	leaq	MACHSTKOFF+MACHSTKSZ(%rdi), %rsp
	// At this point, we are fully in the kernel virtual
	// address space and can discard the identity mapping.
	// To do so, we load the early PML4 that does not have
	// that entry.
	movq	%rax, %cr3			// Also flushes TLB.

	// A pointer to the Mach is already in %rdi, and is the
	// first argument to `main`.  We also pass the multiboot info
	// pointer.
	movq	%rbp, %rsi

	// Push a dummy stack frame and jump to `main`.
	pushq	$0
	xorl	%ebp, %ebp
	leaq	main(%rip), %rax
	pushq	%rax
	pushq	$2				// clear flags
	popfq
	ret
	ud2

// no deposit, no return
// do not resuscitate
.text
.code64
.globl ndnr
ndnr:
	cli
	hlt
	jmp	ndnr

// Start-up IPI handler.
//
// This code is executed on an application processor in response
// to receiving a Start-up IPI (SIPI) from another processor.  The
// vector given in the SIPI determines the memory address the
// where the AP starts execution.
//
// The AP starts in 16-bit real-mode, with:
//  - CS selector set to the startup memory address/16;
//  - CS base set to startup memory address;
//  - CS limit set to 64KiB;
//  - IP set to 0;
//  - CPL set to 0;
//  - A20 latch disabled.
// Welcome to 1978.
//
// The startup memory address is calculated as the vector given in
// the SIPI request, left-shited 12 bits.  Thus, this code must be
// placed on a 4KiB boundary somewhere in low 1 MiB of memory.  We
// have arbitrarily chosen 0x7000, denoted by the symbol `APENTRY`.
//
// While it may seem like this should be in a text section, it is
// deliberately not.  Since this code is copied to APENTRY
// for execution, so as far as the rest of the kernel is concerned,
// the assembled and linked instructions here are just read-only
// data.  So, we put it into .rodata so that it is mapped onto a
// non-executable page and the kernel cannot accidentally jump
// into it once it is running in Rust code on a real page table.
//
// Note that the physical address of the PML4 in the `Mach` for
// this AP is copied into the last 8 bytes of the AP startup page.
// We can use this to establish set the virtual address space for
// this CPU, once we're in long mode.  We use the space immediately
// below that word as a very small stack for getting into long mode.
//
// The 16-bit code loads a basic GDT, turns on 32-bit protected
// mode and makes an inter-segment jump to the protected mode code
// right after.
//
// 32-bit code enables long mode and paging, sets a stack and
// jumps to 64-bit mode, which fixes up virtual addresses for
// the stack and PC and jumps into C.

.set APENTRY,		(7*PGSZ)
.set APMACH,		(APENTRY+PGSZ-8)

.section .rodata

.globl b1978, e1978
.balign 4096
.code16
b1978:
	// We start here in real mode.  Welcome to 1978.
	cli
	cld

	lgdtl	(APENTRY+(apgdtdesc-b1978))

	movl	%cr0, %eax
	orl	$Cr0PE, %eax
	movl	%eax, %cr0

	ljmpl   $GdtCODE32, $(b1982-KZERO)

.balign 16
gdt:
	// 0: Null segment
	.quad	0
	// 8: Kernel 64-bit code segment
	.quad	(SegREAD|SegCODE|SegMB1|SegPRESENT|SegLONG)
	// 16: Kernel 32-bit code segment (for bootstrapping APs)
	.quad	(SegREAD|SegCODE|SegMB1|SegPRESENT|Seg32DEF)
	// 24: Kernel 32-bit data segment (for bootstrapping APs)
	.quad	(SegREAD|SegWRITE|SegMB1|SegPRESENT|Seg32DEF)
egdt:

.balign 16
.skip 6
gdtdesc:
	.word	egdt - gdt - 1
	.long	(gdt - KZERO)

.balign 16
.skip 6
gdtdescv:
	.word	egdt - gdt - 1
	.quad	gdt

.skip 6
apgdtdesc:
.word	egdt - gdt - 1
.long	(APENTRY+gdt-b1978)

.code32
b1982:
	// Protected mode. Welcome to 1982.
	movw	$GdtDATA32, %ax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %ss

	xorl	%eax, %eax
	movw	%ax, %fs
	movw	%ax, %gs

	// In order to fully boot, we need page tables to get into the
	// kernel at its linked addresses, so we reuse the ones used
	// earlier for booting CPU 0.  But note that these also map
	// CPU 0's Mach, and we'll need our own Mach.  While the Mach
	// is mapped at the same virtual address on each CPU, each is
	// obviously different than CPU 0's, and we don't want to
	// trample it.
	//
	// Fortunately, CPU 0 arranges for the physical address of
	// the PML4 for this AP to be at the absolute location APMACH,
	// so we can load our page tables and then set up the pointer
	// to our own Mach.
	//
	// We also use the space below APMACH as a (very) small stack
	// for the jump to our linked address.
	movl	$(bootpml4 - KZERO), %eax
	movl	%eax, %cr3

	// Enable and activate Long Mode.
	movl	%cr4, %eax
	andl	$~Cr4PSE, %eax		// PSE always true in long mode
	// Enable page global, physical address extension, and FS/GS base
	// load/store instruction support.
	orl	$(Cr4PGE|Cr4PAE|Cr4FSGSBASE), %eax
	movl	%eax, %cr4

	movl	$IA32_EFER, %ecx	// Extended Feature Enable
	rdmsr
	orl	$(EferSCE|EferLME|EferNXE), %eax
	wrmsr				// Long Mode Enable

	movl	%cr0, %edx
	andl	$~(Cr0CD|Cr0NW|Cr0TS|Cr0MP), %edx
	orl	$(Cr0PG|Cr0WP), %edx	// Paging Enable
	movl	%edx, %cr0

	ljmp	$GdtCODE64, $(1f-KZERO)

.code64
1:
	movabsq	$gdtdescv, %rax
	lgdt	(%rax)
	pushq	$GdtCODE64
	pushq	$(1f-KZERO)
	lretq
1:
	xorl	%eax, %eax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %fs
	movw	%ax, %gs
	movw	%ax, %ss

	leaq	APMACH, %rdi
	movq	%rdi, %rsp
	movabsq	$apwarp64, %rax
	pushq	%rax
	ret
	ud2

apwarp64:
	// Move to the real PML4 from our Mach.
	movq	(%rdi), %rax
	movq	%rax, %cr3

	// Load the address of our Mach as the argument for squidboy,
	// and load our real stack pointer from the Mach.
	movabsq	$(KCPUZERO+KMACH), %rdi
	leaq	MACHSTKOFF+MACHSTKSZ(%rdi), %rsp

	pushq	$0
	xorl	%ebp, %ebp
	movq	128*KiB+8(%rdi), %rax			// m->scratch
	pushq	%rax
	pushq	$2				// Clear flags
	popfq
	ret					// Call squidboy
	ud2

e1978:

.balign 4096
bootpml4:
	.quad	bootidentitypt3 - KZERO + (PteRW | PteP)
	.space	4096 - 3*8
	.quad	bootmachpt3 - KZERO + (PteRW | PteP)
	.quad	bootkernpt3 - KZERO + (PteRW | PteP)

bootidentitypt3:
	.quad	(0<<30) + (PtePS | PteRW | PteP)
	.quad	(1<<30) + (PtePS | PteRW | PteP)
	.quad	(2<<30) + (PtePS | PteRW | PteP)
	.quad	(3<<30) + (PtePS | PteRW | PteP)
	.space	4096 - 4*8

bootmachpt3:
	// The CPU part
	.quad	(0<<30) + (PtePS | PteRW | PteP)
	.space	4096/2 - 1*8
	// The node part
	.quad	(0<<30) + (PtePS | PteRW | PteP)
	.space	4096/2 - 1*8

bootkernpt3:
	.space	4096 - 2*8
	.quad	(0<<30) + (PtePS | PteRW | PteP)
	.quad	0
