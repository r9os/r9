OUTPUT_ARCH(${ARCH})
ENTRY(start)

SECTIONS {
	. = ${LOAD-ADDRESS};
	.text : ALIGN(4096) {
		*(.text.entry)
		*(.text*)
		. = ALIGN(2097152);
		PROVIDE(etext = .);
	}

	.rodata : ALIGN(4096) {
		*(.rodata*)
		*(.srodata*)
		. = ALIGN(2097152);
		PROVIDE(erodata = .);
	}

	.data : ALIGN(4096) {
		*(.data*)
		*(.sdata*)
		. = ALIGN(2097152);
		PROVIDE(edata = .);
	}

	.bss : ALIGN(4096) {
		*(.bss*)
		*(.sbss*)
		*(COMMON)
		. = ALIGN(2097152);
		PROVIDE(end = .);
	}

	/DISCARD/ : {
		*(.eh_frame)
	}
}
