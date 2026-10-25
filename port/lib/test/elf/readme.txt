This folder contains test files for the ELF code in the elf and exec modules.

- hello.elf: built from hello.rs and hello.ld by build.sh.  A static aarch64
  executable with three segments at 0x10000: text (R+X), rodata (R), and data
  (R+W), whose two pages of file bytes are followed by a bss that runs onto
  a third page.  The kernel runs it in QEMU.
