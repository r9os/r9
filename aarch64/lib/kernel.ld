/*
 * Linker script for R9.
 */

ENTRY(start)

SECTIONS {
	/* Entrypoint for Raspberry Pi will be at 0x80000 */
	. = ${LOAD-ADDRESS};

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
	ebss = .;

	heap = .;
	.heap : ALIGN(4096) {
		. = ALIGN(8*2097152);
	}
	eheap = .;

	/* Reserve section for early pagetables.  Align to 2MiB to allow us to map
	   as a 2MiB page.Note that this won't be needed once we transition to
	   recursive pagetables.
	   TODO Just use the heap when we enable recursive pagetables? */
	. = ALIGN(2 * 1024 * 1024);
	early_pagetables = .;
	. += 2 * 1024 * 1024;
	eearly_pagetables = .;

	end = .;
	PROVIDE(end = .);
	
	/DISCARD/ : {
		*(.eh_frame .note.GNU-stack)
	}
}
