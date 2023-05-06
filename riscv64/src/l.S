.section .text.entry
.globl start
start:
	// a0 == hartid
	// pc == 0x80200000
	// sp == 0x800xxxxx

	// 1. set sp
	// sp = bootstack + (hartid + 1) * 0x10000
	add		t0, a0, 1
	slli	t0, t0, 16 // ??? this is  * 0x4000, not 0x10000
	lui		sp, %hi(bootstack)
	add	  sp, sp, t0 // add stack length

	// enable paging
	// satp = (8 << 60) | PPN(boot_page_table)
	// SATP = Supervisor Address Translation and Protection
	// Register definition:
	// |  MODE  |  ASID  |  PPN  |
	// |[63..60]|[59..44]|[43..0]|
	// PPN = Physical Page Number (of root page table)
	// ASID = Address Space Identifier
	// Volume II: RISC-V Privileged Architectures V20211203 p75
	// SXLEN 64, MODE 8: Sv39 aka Page-based 39-bit virtual addressing
	lui     t0, %hi(boot_page_table)
	li      t1, 0xffffffffc0000000 - 0x80000000
	sub     t0, t0, t1
	srli    t0, t0, 12 // page size 4k (2^12)
	li      t1, 8 << 60 // mode Sv39
	or      t0, t0, t1
	csrw    satp, t0
	sfence.vma

	// 3. jump to main9 (absolute address)
	lui     t0, %hi(main9)
	addi    t0, t0, %lo(main9)
	jr      t0

1:
	wfi
	j	1b

/* STACK */
.section .bss.stack
.align 12   # page align
.global bootstack
bootstack:
.space 4096 * 4 * 8

.global bootstacktop
bootstacktop:
.section .data
.align 12 // page align

/* PAGING */
boot_page_table:
.global boot_page_table
// PTE (Page Table Entry) definition (SV39)
// NOTE: A page table entry is 8 bytes; 256 entries are 512*8=4k.
//
// N is reserved for Svnapot, PBMT is Page-based Memory Types
//     |  N |  PBMT  | Rsvd.  |
//     |[63]|[62..61]|[60..54]|
// The Physical Page Number (PPN) is 9+9+26 bits long,
// mapped to a 9+9+9 bit Virtual Page Number (VPN).
//             | PPN[2] | PPN[1] | PPN[0] |
//             |[53..28]|[27..19]|[18..10]|
// RSW is reserved for supervisor software control.
// D - dirty, A - accessed, G - global mapping, U - user mode access,
// XWR - exec/write/read (table 4.5, p80), V - entry is valid
//                     | RSW  | D | A | G | U | X | W | R | V |
//                     |[9..8]|[7]|[6]|[5]|[4]|[3]|[2]|[1]|[0]|
//
// NOTE: quad = 4 words = 4 double bytes = 8 bytes
// https://ftp.gnu.org/old-gnu/Manuals/gas-2.9.1/html_chapter/as_7.html#SEC115
// 0x00000000_80000000 -> 0x80000000 (1G)
// 0xffffffff_c0000000 -> 0x80000000 (1G)
.quad 0
.quad 0
.quad (0x80000 << 10) | 0xcf // VRWXAD
.zero 8 * 508
.quad (0x80000 << 10) | 0xcf // VRWXAD
