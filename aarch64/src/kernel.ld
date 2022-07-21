/*
 * Linker script for R9.
 */

ENTRY(start)

SECTIONS {
	/*
	 * Raspberry Pi start address is 0x80_0000.
	 */
	/* . = 0xffff800000100000; */

	. = 0x80000;

	PROVIDE(boottext = .);
	.text.boot : ALIGN(4096) {
		*(.boottext .bootdata)
		. = ALIGN(4096);
		PROVIDE(eboottext = .);
		. = ALIGN(2097152);
		PROVIDE(esys = .);
	}

	PROVIDE(text = .);
	.text : ALIGN(4096) {
		*(.text* .stub .gnu.linkonce.t.*)
		. = ALIGN(2097152);	
		PROVIDE(etext = .);
	}

	.rodata : ALIGN(4096) {
		*(.rodata* .gnu.linkonce.r.*)
		. = ALIGN(2097152);
		PROVIDE(erodata = .);
	}

	.data : ALIGN(4096) {
		*(.data*)
	}
	.got : ALIGN(4096) {
		*(.got)
	}
	.got.plt : ALIGN(4096) {
		*(.got.plt)
	}
	PROVIDE(edata = .);

	PROVIDE(bss = .);
	.bss : ALIGN(4096) {
		*(.bss*)
		*(COMMON)
		. = ALIGN(2097152);
	}
	PROVIDE(end = .);

	/DISCARD/ : {
		*(.eh_frame .note.GNU-stack)
	}
}
