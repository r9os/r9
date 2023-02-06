OUTPUT_ARCH(riscv)
ENTRY(start)

SECTIONS {
	/*
	 * XXX This needs to change to a virtual address
	 * once we set up paging.
	 *
	 * OpenSBI expects us to start at 0x80200000 
	 */
	. = 0x80200000;
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
