// It gets ugly to try to link this at some low address
// and then have the rest of the kernel linked high; that
// goes doubly for any attempt to load at a random address.
//
// So you have to learn to write position independent
// code here.
//
// It will make you stronger.
//
// Assuming you survive the training.

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

.set PTSZ,			4096
.set PGSZ,			4096
.set MACHSTKSZ,			(8 * PGSZ)

.set KZERO,			0xffff800000000000
.set MiB,			(1 << 20)
.set KSYS,			(KZERO+MiB+PGSZ)
.set KTZERO,			(KZERO+2*MiB)

/* Cr0 */
.set Pe, 0x00000001 /* Protected Mode Enable */
.set Mp, 0x00000002 /* Monitor Coprocessor */
.set Em, 0x00000004 /* Emulate Coprocessor */
.set Ts, 0x00000008 /* Task Switched */
.set Et, 0x00000010 /* Extension Type */
.set Ne, 0x00000020 /* Numeric Error  */
.set Wp, 0x00010000 /* Write Protect */
.set Am, 0x00040000 /* Alignment Mask */
.set Nw, 0x20000000 /* Not Writethrough */
.set Cd, 0x40000000 /* Cache Disable */
.set Pg, 0x80000000 /* Paging Enable */

/* Cr3 */
.set Pwt, 0x00000008 /* Page-Level Writethrough */
.set Pcd, 0x00000010 /* Page-Level Cache Disable */

/* Cr4 */
.set Vme, 0x00000001        /* Virtual-8086 Mode Extensions */
.set Pvi, 0x00000002        /* Protected Mode Virtual Interrupts */
.set Tsd, 0x00000004        /* Time-Stamp Disable */
.set De, 0x00000008         /* Debugging Extensions */
.set Pse, 0x00000010        /* Page-Size Extensions */
.set Pae, 0x00000020        /* Physical Address Extension */
.set Mce, 0x00000040        /* Machine Check Enable */
.set Pge, 0x00000080        /* Page-Global Enable */
.set Pce, 0x00000100        /* Performance Monitoring Counter Enable */
.set Osfxsr, 0x00000200     /* FXSAVE/FXRSTOR Support */
.set Osxmmexcpt, 0x00000400 /* Unmasked Exception Support */

/* MSRs */
.set Efer, 0xc0000080         /* Extended Feature Enable */
.set Star, 0xc0000081         /* Legacy Target IP and [CS]S */
.set Lstar, 0xc0000082        /* Long Mode Target IP */
.set Cstar, 0xc0000083        /* Compatibility Target IP */
.set Sfmask, 0xc0000084       /* SYSCALL Flags Mask */
.set FSbase, 0xc0000100       /* 64-bit FS Base Address */
.set GSbase, 0xc0000101       /* 64-bit GS Base Address */
.set KernelGSbase, 0xc0000102 /* SWAPGS instruction */

/* Efer */
.set Sce, 0x00000001   /* System Call Extension */
.set Lme, 0x00000100   /* Long Mode Enable */
.set Lma, 0x00000400   /* Long Mode Active */
.set Nxe, 0x00000800   /* No-Execute Enable */
.set Svme, 0x00001000  /* SVM Extension Enable */
.set Ffxsr, 0x00004000 /* Fast FXSAVE/FXRSTOR */

/* PML4E/PDPE/PDE/PTE */
.set PteP, 0x0000000000000001     /* Present */
.set PteRW, 0x0000000000000002    /* Read/Write */
.set PteU, 0x0000000000000004     /* User/Supervisor */
.set PtePWT, 0x0000000000000008   /* Page-Level Write Through */
.set PtePCD, 0x0000000000000010   /* Page Level Cache Disable */
.set PteA, 0x0000000000000020     /* Accessed */
.set PteD, 0x0000000000000040     /* Dirty */
.set PtePS, 0x0000000000000080    /* Page Size */
.set Pte4KPAT, PtePS              /* PTE PAT */
.set PteG, 0x0000000000000100     /* Global */
.set Pte2MPAT, 0x0000000000001000 /* PDE PAT */
.set Pte1GPAT, Pte2MPAT           /* PDPE PAT */
.set PteNX, 0x8000000000000000ULL /* No Execute */

.align 4
.section .boottext, "awx"
multiboot_header:
.long MULTIBOOT_MAGIC
.long MULTIBOOT_FLAGS
.long MULTIBOOT_CHECKSUM

// When we get here we are in protected mode with a GDT.  We set
// up IA32e mode and get into long mode with paging enabled.
.code32
.align 4
.globl start
start:
	cli
	cld

	// Save the multiboot magic number.
	movl	%eax, %ebp

	// Make the basic page tables for CPU0 to map 0-4GiB
	// physical to KZERO, in addition to an identity map
	// for the switch from protected to paged mode.  There
	// is an assumption here that the creation and later
	// removal of the identity map will not interfere with
	// the KZERO mappings.
	//
	// We assume a recent processor with Page Size Extensions
	// and use two 2MiB entries.

	// Zero the stack, page tables, vsvm, unused pages, m, sys, etc.
	movl	$(KSYS-KZERO), %esi
	movl	$((KTZERO-KSYS)/4), %ecx
	xorl	%eax, %eax
	movl	%esi, %edi
	rep stosl

	// We could zero the BSS here, but the loader does it for us.

	// Set the stack and find the start of the page tables.
	movl	%esi, %eax
	addl	$MACHSTKSZ, %eax
	movl	%eax, %esp			// Give ourselves a stack

	// %eax points to the PML4 that we'll use for double-mapping
	// low RAM and KZERO.
	movl	%eax, %cr3			// load the MMU; paging still disabled
	movl	%eax, %edx
	addl	$(2*PTSZ|PteRW|PteP), %edx	// EPML3 at IPML4 + 2*PTSZ
	movl	%edx, (%eax)			// IPML4E for identity map
	movl	%edx, 2048(%eax)		// IPML4E for KZERO

	// The next page frame contains a PML4 that removes the double
	// mapping, leaving only KZERO mapped.
	addl	$PTSZ, %eax			// EPML4 at IPML4 + PTSZ
	movl	%edx, 2048(%eax)		// EPML4E for EMPL3 at KZERO

	// Fill in the early PML3 (PDPT) to point the early PML2's (PDs)
	// that provide the initial 4GiB mapping in the kernel.
	addl	$PTSZ, %eax			// EPML3 at EPML4 + PTSZ
	addl	$PTSZ, %edx			// EPML2[0] at EPML3 + PTSZ
	movl	%edx, (%eax)			// EPML3E for EPML2[0]
	addl	$PTSZ, %edx			// EPML2[1] at EPML2[0] + PTSZ
	movl	%edx, 8(%eax)			// EPML3E for EPML2[1]
	addl	$PTSZ, %edx			// EPML2[2] at EPML2[1] + PTSZ
	movl	%edx, 16(%eax)			// EPML3E for EPML2[2]
	addl	$PTSZ, %edx			// EPML2[3] at EPML2[2] + PTSZ
	movl	%edx, 24(%eax)			// EPML3E for EPML2[3]

	// Map the first 4GiB (the entire 32-bit) address space.
	// Note that this requires 16KiB.
	//
	// The first 2MiB are mapped using 4KiB pages.  The first 1MiB
	// memory contains holes for MMIO and ROM and other things that
	// we want special attributes for.  We'll set those in the
	// kernel proper, but we provide 4KiB pages here.  There is 4KiB
	// of RAM for the PT immediately after the PDs.
	addl	$PTSZ, %eax			// PML2[0] at PML3[0] + PTSZ
	movl	$2048, %ecx			// 2048 * 2MiB pages covers 4GiB
	movl	$(PtePS|PteRW|PteP), %edx	// Large page PDEs
1:	movl	%edx, (%eax)			// PDE for 2MiB pages
	addl	$8, %eax
	addl	$(2<<20), %edx
	subl	$1, %ecx
	test	%ecx, %ecx
	jnz	1b

	// %eax now points to the page after the EPML2s, which is the real
	// self-referential PML4.
	// Map the first 192 entries for the upper portion of the address
	// to PML3s; this is the primordial root of sharing for the kernel.
	movl	%eax, %edx
	addl	$(PTSZ|PteRW|PteP), %edx	// PML3[0] at PML4 + PTSZ
	movl	$256, %ecx
1:	movl	%edx, (%eax, %ecx, 8)
	addl	$PTSZ, %edx
	incl	%ecx
	cmp	$(256+192), %ecx
	jne	1b

	// Enable and activate Long Mode.  From the manual:
	// make sure Page Size Extentions are off, and Page Global
	// Extensions and Physical Address Extensions are on in CR4;
	// set Long Mode Enable in the Extended Feature Enable MSR;
	// set Paging Enable in CR0;
	// make an inter-segment jump to the Long Mode code.
	// It`s all in 32-bit mode until the jump is made.
	movl	%cr4, %eax
	andl	$~Pse, %eax			// Page Size
	orl	$(Pge|Pae), %eax		// Page Global, Phys. Address
	movl	%eax, %cr4

	movl	$Efer, %ecx			// Extended Feature Enable
	rdmsr
	orl	$Lme, %eax			// Long Mode Enable
	orl	$Nxe, %eax			// Long Mode Enable
	wrmsr

	movl	%cr0, %edx
	andl	$~(Cd|Nw|Ts|Mp), %edx
	orl	$(Pg|Wp), %edx			// Paging Enable
	movl	%edx, %cr0

	// Load the 64-bit GDT
	movl	$(gdtdesc-KZERO), %eax
	lgdt	(%eax)

	ljmpl	$GdtCODE64, $(1f-KZERO)

.code64
1:
	// Long mode. Welcome to 2003.  Jump out of the identity map
	// and into the kernel address space.

	// Load a 64-bit GDT in the kernel address space.
	movabsq	$gdtdescv, %rax
	lgdt	(%rax)

	// Zero out the segment registers: they are not used in long mode.
	xorl	%edx, %edx
	movw	%dx, %ds
	movw	%dx, %es
	movw	%dx, %fs
	movw	%dx, %gs
	movw	%dx, %ss

	// We can now use linked addresses for the stack and code.
	// We'll jump into the kernel from here.
	movabsq	$KZERO, %rax
	addq	%rax, %rsp
	movabsq	$warp64, %rax
	jmp	*%rax

.text
.code64
warp64:
	// At this point, we are fully in the kernel virtual
	// address space and we can discard the identity mapping.
	// There is a PML4 sans identity map 4KiB beyond the
	// current PML4; load that, which also flushes the TLB.
	movq	%cr3, %rax
	addq	$PTSZ, %rax
	movq	%rax, %cr3			// Also flushes TLB.

	// &sys->mach is the first argument to main()
	movabsq	$KSYS, %rdi
	addq	$(MACHSTKSZ+(1+1+1+4+1+192)*PTSZ+PGSZ), %rdi
	movq	%rbp, %rsi			// multiboot magic
	movq	%rbx, %rdx			// multiboot info pointer

	// Push a dummy stack frame and jump to `main`.
	pushq	$0
	movq	$0, %rbp
	leaq	main9(%rip), %rax
	push	%rax
	pushq	$2				// clear flags
	popfq
	ret
	ud2

// no deposit, no return
// do not resuscitate
.globl ndnr
ndnr:
	sti
	hlt
	jmp	ndnr

// Start-up request IPI handler.
//
// This code is executed on an application processor in response
// to receiving a Start-up IPI (SIPI) from another processor.  The
// vector given in the SIPI determines the memory address the
// where the AP starts execution.
//
// The AP starts in real-mode, with
//   CS selector set to the startup memory address/16;
//   CS base set to startup memory address;
//   CS limit set to 64KiB;
//   CPL and IP set to 0.
//
// This must be placed on a 4KiB boundary, and while it may seem
// like this should be in a text section, it is deliberately not.
// The AP entry code is copied to a page in low memory at APENTRY
// for execution, so as far as the rest of the kernel is concerned
// it is simply read-only data.  We put it into .rodata so that it
// is mapped onto a non-executable page and the kernel cannot
// accidentally jump into it once it is running in C code on a
// real page table.
//
// The 16-bit code loads a basic GDT, turns on 32-bit protected
// mode and makes an inter-segment jump to the protected mode code
// right after.
//
// 32-bit code enables long mode and paging, sets a stack and
// jumps to 64-bit mode, which fixes up virtual addresses for
// the stack and PC and jumps into C.

.set APENTRY,		0x3000
.set APPERCPU,		(0x4000-8)

.section .rodata

.globl b1978, e1978
.code16
.align 4096
b1978:
	// We start here in real mode.  Welcome to 1978.
	cli
	cld

	lgdtl	(APENTRY+(apgdtdesc-b1978))

	movl	%cr0, %eax
	orl	$Pe, %eax
	movl	%eax, %cr0

	ljmpl   $GdtCODE32, $(b1982-KZERO)

.align 16
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

.skip 6
apgdtdesc:
.word	egdt - gdt - 1
.long	(APENTRY+gdt-b1978)

e1978:

.text
.code32
b1982:
	// Protected mode. Welcome to 1982.
	movw	$GdtDATA32, %ax
	movw	%ax, %ds
	movw	%ax, %es
	movw	%ax, %fs
	movw	%ax, %gs
	movw	%ax, %ss

	// load the PML4 with the shared page table address;
	// make an identity map for the inter-segment jump below,
	// using the stack space to hold a temporary PDP and PD;
	// enable and activate long mode;
	// make an inter-segment jump to the long mode code.
	movl	$(KSYS-KZERO+MACHSTKSZ), %eax	// Page table
	movl	%eax, %cr3			// load the mmu

	// Enable and activate Long Mode.
	movl	%cr4, %eax
	andl	$~Pse, %eax			// Page Size
	orl	$(Pge|Pae), %eax		// Page Global, Phys. Address
	movl	%eax, %cr4

	movl	$Efer, %ecx			// Extended Feature Enable
	rdmsr
	orl	$Lme, %eax			// Long Mode Enable
	orl	$Nxe, %eax			// Long Mode Enable
	wrmsr

	movl	%cr0, %edx
	andl	$~(Cd|Nw|Ts|Mp), %edx
	orl	$(Pg|Wp), %edx			// Paging Enable
	movl	%edx, %cr0

	ljmp	$GdtCODE64, $(1f-KZERO)

.code64
1:
	movq	APPERCPU, %rdi
	addq	$MACHSTKSZ, %rdi
	movq	%rdi, %rsp			// set stack
	addq	$(PTSZ+PGSZ), %rdi		// Mach *

	movabsq	$apwarp64, %rax
	pushq	%rax
	ret
	ud2

apwarp64:
	movabsq	$gdtdescv, %rax
	lgdt	(%rax)

	xorl	%edx, %edx
	movw	%dx, %ds
	movw	%dx, %es
	movw	%dx, %fs
	movw	%dx, %gs
	movw	%dx, %ss

	movq	%cr3, %rax
	addq	$(7*PTSZ), %rax
	movq	%rax, %cr3			// flush TLB

	pushq	$0
	movq	$0, %rbp
	movq	8(%rdi), %rax			// m->splpc
	pushq	%rax
	pushq	$2				// Clear flags
	popfq
	ret					// Call squidboy
	ud2

.section .rodata

.align 16
.skip 6
gdtdesc:
	.word	egdt - gdt - 1
	.long	(gdt - KZERO)

.align 16
.skip 6
gdtdescv:
	.word	egdt - gdt - 1
	.quad	gdt
