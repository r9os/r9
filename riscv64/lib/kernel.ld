OUTPUT_ARCH(${ARCH})
ENTRY(start)

SECTIONS {
	. = ${LOAD-ADDRESS};

	text = .;
	.text : ALIGN(4096) {
		*(.text.entry)
		*(.text*)
		. = ALIGN(2097152);
		etext = .;
	}

	rodata = .;
	.rodata : ALIGN(4096) {
		*(.rodata*)
		. = ALIGN(2097152);
		erodata = .;
	}

	data = .;
	.data : ALIGN(4096) {
		*(.data*)
		. = ALIGN(2097152);
		edata = .;
	}

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
