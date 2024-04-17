// 0xFFFFFFFF40000000
.equ PHYSICAL_MEMORY_OFFSET, (0xFFFFFFFFC0000000 - 0x80000000)

.section .text.entry
.globl start
start:
	// a0 == hartid
	// pc == 0x80200000
	// sp == 0x800xxxxx

	// 1. set sp
	// sp = bootstack + (hartid + 1) * 0x10000
	add		t0, a0, 1
	slli	t0, t0, 16
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
  // page size is 4k (2^12) - drop lowest 12 bits
  srli    t4, t0, 12

  // subtract offset to get the physical address of the root page table
  li      t1, PHYSICAL_MEMORY_OFFSET
  sub     t2, t0, t1

  // page size is 4k (2^12) - drop lowest 12 bits
  srli    t3, t2, 12
  // final SATP
  li      t1, 8 << 60 // mode Sv39
  or      t0, t3, t1

  // t3 holds the root page table physical address divided by 4k; shift for flags
  slli    t1, t4, 10
  ori     t1, t4, 0xc1 // DAGU XWRV
  sd      t1, 8*255(t2) // #255

  // flush TLB
  sfence.vma
  csrw    satp, t0
  // flush TLB
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
// PTE (Page Table Entry) definition (SV39)
// A virtual address (VA) in SV39 consists of 3 parts plus the final offset.
// Those parts make up the _virtual page number_ (VPN).
// Each VPN part is 9 bits long, used to point to an entry in a page table.
// The VA offset is another 12 bits, so that it can point to any of 4K bytes.
// 9 + 9 + 9 + 12 = 39
//
// NOTE: A page table entry is 8 bytes; 512 entries are 512*8=4k.
//
// N is reserved for Svnapot, PBMT is Page-based Memory Types
//     |  N |  PBMT  | Rsvd.  |
//     |[63]|[62..61]|[60..54]|
// The Physical Page Number (PPN) is 26 + 9 + 9 = 44 bits long.
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
// Those are leaf pages (at least on of RWX set), 1G (0x4000_0000) each.

// Identity map in the beginning
//       1st page: 0x00000000_00000000 -> 0x00000000 (1G)
.quad (0x00000 << 10) | 0xcf // VRWXAD
//       2nd page: 0x00000000_40000000 -> 0x40000000 (1G)
.quad (0x40000 << 10) | 0xcf // VRWXAD
//       3rd page: 0x00000000_80000000 -> 0x80000000 (1G)
.quad (0x80000 << 10) | 0xcf // VRWXAD
// 504 empty entries

// Remap first 3GB in the end
.zero 506 * 8
//  3rd last page: 0xffffffff_40000000 -> 0x00000000 (1G)
.quad (0x00000 << 10) | 0xcf // VRWXAD
//  2nd last page: 0xffffffff_80000000 -> 0x40000000 (1G)
.quad (0x40000 << 10) | 0xcf // VRWXAD
// very last page: 0xffffffff_c0000000 -> 0x80000000 (1G)
.quad (0x80000 << 10) | 0xcf // VRWXAD
