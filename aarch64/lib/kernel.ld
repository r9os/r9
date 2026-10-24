/*
 * Linker script for R9.
 */

ENTRY(start)
KZERO = 0xffff800000000000;

SECTIONS {
	/* Entrypoint for Raspberry Pi will be at 0x80000 */
	. = ${LOAD-ADDRESS};


	/* Group boottext and text */
	.text.boot : AT (ADDR(.text.boot) - KZERO) {
		*(.boottext .bootdata)
		. = ALIGN(4096);
	}
	PROVIDE(boottext_pa = LOADADDR(.text.boot));
	PROVIDE(eboottext_pa = LOADADDR(.text.boot) + SIZEOF(.text.boot));


	. = ALIGN(4096);
	.text : AT (ADDR(.text) - KZERO) {
		*(.text* .stub .gnu.linkonce.t.*)
		. = ALIGN(2097152);
	}
	PROVIDE(text_pa = LOADADDR(.text));
	PROVIDE(etext_pa = LOADADDR(.text) + SIZEOF(.text));


	/* RO data is in a separate page to other data */
	. = ALIGN(2097152);
	.rodata : ALIGN(2097152) {
		*(.rodata* .gnu.linkonce.r.*)
		. = ALIGN(2097152);
	}
	PROVIDE(rodata_pa = LOADADDR(.rodata));
	PROVIDE(erodata_pa = LOADADDR(.rodata) + SIZEOF(.rodata));


	/* Group data and bss */
	.data : ALIGN(2097152) {
		*(.data*)
		*(.got)
		*(.got.plt)
		. = ALIGN(4096);
	}
	PROVIDE(data_pa = LOADADDR(.data));
	PROVIDE(edata_pa = LOADADDR(.data) + SIZEOF(.data));


	.bss : ALIGN(4096) {
		*(.bss*)
		*(COMMON)
		. = ALIGN(4096);
	}

	/* Reserve section for early pagetables. */
	.earlyvm_pagetables : ALIGN(4096) {
		. += 32 * 4096;
	}
	PROVIDE(earlyvm_pagetables_pa = LOADADDR(.earlyvm_pagetables));
	PROVIDE(eearlyvm_pagetables_pa = LOADADDR(.earlyvm_pagetables) + SIZEOF(.earlyvm_pagetables));


/* TODO Should this be here???  Better to put into the bss section later */
	.end : ALIGN(2097152) {}

	PROVIDE(bss_pa = LOADADDR(.bss));
	PROVIDE(ebss_pa = LOADADDR(.end));


	/DISCARD/ : {
		*(.eh_frame .eh_frame_hdr .note.GNU-stack)
	}
}
