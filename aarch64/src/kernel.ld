/*
 * Linker script for R9.
 */

ENTRY(start)

SECTIONS {
	/* Entrypoint for Raspberry Pi will be at 0x80000 */
	. = 0xffff800000100000 - 0x80000;

	boottext = .;
	.text.boot : ALIGN(4096) {
		*(.boottext .bootdata)
		. = ALIGN(4096);
		eboottext = .;
		. = ALIGN(2097152);
		esys = .;
	}

	text = .;
	.text : ALIGN(4096) {
		*(.text* .stub .gnu.linkonce.t.*)
		. = ALIGN(2097152);	
		etext = .;
	}

	rodata = .;
	.rodata : ALIGN(4096) {
		*(.rodata* .gnu.linkonce.r.*)
		. = ALIGN(2097152);
		erodata = .;
	}

	data = .;
	.data : ALIGN(4096) {
		*(.data*)
	}
	.got : ALIGN(4096) {
		*(.got)
	}
	.got.plt : ALIGN(4096) {
		*(.got.plt)
	}
	edata = .;

	bss = .;
	.bss : ALIGN(4096) {
		*(.bss*)
		*(COMMON)
		. = ALIGN(2097152);
	}
	end = .;

	/DISCARD/ : {
		*(.eh_frame .note.GNU-stack)
	}
}
