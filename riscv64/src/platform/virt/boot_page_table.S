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
