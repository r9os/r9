/*
 * Linker script for R9.
 */

ENTRY(start)
KZERO = 0xffffffff80000000;

SECTIONS {
	/*
	 * start the kernel at 0x0xffff_ffff_8020_0000
	 * This preserves some RAM between 1MiB and the
	 * start of the kernel for critical structures.
	 */
	. = ${LOAD-ADDRESS};
	PROVIDE(text = .);
	.text : AT (ADDR(.text) - KZERO) {
		*(.text.boot)
		*(.*text* .stub .gnu.linkonce.t.*)
		. = ALIGN(4096);
		*(.trap*)
		. = ALIGN(2097152);
		PROVIDE(etext = .);
	}

	.rodata : AT (ADDR(.rodata) - KZERO) {
		*(.*rodata* .gnu.linkonce.r.*)
		. = ALIGN(2097152);
		PROVIDE(erodata = .);
	}

	.data : AT (ADDR(.data) - KZERO) {
		*(.*data*)
		. = ALIGN(4096);
	}
	PROVIDE(edata = .);

	.bss : AT (ADDR(.bss) - KZERO) {
		*(COMMON)
		*(.*bss*)
		. = ALIGN(2097152);
	}
	PROVIDE(end = .);

	/DISCARD/ : {
		*(.eh_frame* .note* .got*)
	}
}
