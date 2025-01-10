/*
 * Linker script for R9.
 */

ENTRY(start)

SECTIONS {
	/* Entrypoint for Raspberry Pi will be at 0x80000 */
	. = ${LOAD-ADDRESS};


	/* Group boottext and text */
	PROVIDE(boottext = .);
	.text.boot : ALIGN(4096) {
		*(.boottext .bootdata)
	}
	. = ALIGN(4096);
	PROVIDE(eboottext = .);

	PROVIDE(text = .);
	.text : ALIGN(4096) {
		*(.text* .stub .gnu.linkonce.t.*)
	}
	. = ALIGN(2097152);
	PROVIDE(etext = .);


	/* RO data is in a separate page to other data */
	PROVIDE(rodata = .);
	.rodata : ALIGN(2097152) {
		*(.rodata* .gnu.linkonce.r.*)
	}
	. = ALIGN(2097152);
	PROVIDE(erodata = .);


	/* Group data and bss */
	PROVIDE(data = .);
	.data : ALIGN(4096) {
		*(.data*)
	}
	.got : ALIGN(4096) {
		*(.got)
	}
	.got.plt : ALIGN(4096) {
		*(.got.plt)
	}
	. = ALIGN(4096);
	PROVIDE(edata = .);

	PROVIDE(bss = .);
	.bss : ALIGN(4096) {
		*(.bss*)
		*(COMMON)
	}
	. = ALIGN(4096);
	PROVIDE(ebss = .);


	/* Reserve section for early pagetables. */
	. = ALIGN(32 * 4096);
    early_pagetables = .;
	. += 32 * 4096;
	eearly_pagetables = .;


	PROVIDE(end = .);

	/DISCARD/ : {
		*(.eh_frame .note.GNU-stack)
	}
}
