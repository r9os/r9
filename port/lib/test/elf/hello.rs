//! The ELF loader's test program.  Its data spans two pages.  It checks its
//! data, that every word of its bss is zero, and that brk_(0) is the start of
//! its bss, then writes a message from its rodata and exits.  It exits with
//! "elf" if any check fails.

#![no_std]
#![no_main]

core::arch::global_asm!(
    ".section .text._start, \"ax\"",
    ".global _start",
    "_start:",
    // Both words of the data, a page apart, have their values.
    "   adr x19, word",
    "   ldr x0, [x19]",
    "   ldr x1, 3f",
    "   cmp x0, x1",
    "   b.ne 1f",
    "   adr x19, word2",
    "   ldr x0, [x19]",
    "   mvn x1, x1",
    "   cmp x0, x1",
    "   b.ne 1f",
    // Every word of the bss is zero.
    "   adr x20, bss",
    "   adr x21, bss_end",
    "   mov x22, x20",
    "5: ldr x0, [x22], #8",
    "   cbnz x0, 1f",
    "   cmp x22, x21",
    "   b.lo 5b",
    // brk_(0)
    "   mov x0, #0",
    "   mov x8, #24",
    "   svc #0",
    "   cmp x0, x20",
    "   b.ne 1f",
    // pwrite(1, msg, len, -1)
    "   mov x0, #1",
    "   adr x1, msg",
    "   mov x2, #(msg_end - msg)",
    "   mov x3, #-1",
    "   mov x8, #51",
    "   svc #0",
    "   cmp x0, #(msg_end - msg)",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 4f",
    // exits
    "2: mov x8, #8",
    "   svc #0",
    ".balign 8",
    "3: .quad 0x1234567890abcdef",
    "4: .asciz \"elf\"",
    ".section .rodata.msg, \"a\"",
    "msg: .ascii \"hello from elf\\n\"",
    "msg_end:",
    ".section .data.word, \"aw\"",
    ".balign 8",
    "word: .quad 0x1234567890abcdef",
    ".fill 4096 - 8, 1, 0x5a",
    "word2: .quad ~0x1234567890abcdef",
    ".section .bss.buf, \"aw\", @nobits",
    ".balign 8",
    "bss: .zero 5120",
    "bss_end:",
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
