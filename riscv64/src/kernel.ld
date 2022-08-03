OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
	/*
	 * OpenSBI expects us to start at 0x80200000 
	 */

	. = 0x80200000;

	.text : {
		*(.text.entry)
		. = ALIGN(4K);
		*(.text .text.*)
	}

	. = ALIGN(4K);

	srodata = .;
	.rodata : {
		*(.rodata .rodata.*)
		*(.srodata .srodata.*)
	}

	. = ALIGN(4K);
	erodata = .;
	sdata = .;
	.data : {
		*(.data .data.*)
		*(.sdata .sdata.*)
	}

	. = ALIGN(4K);
	edata = .;
	.bss : {
		sbss = .;
		*(.bss .bss.*)
		*(.sbss .sbss.*)
	}

	. = ALIGN(4K);
	ebss = .;

	/DISCARD/ : {
		*(.eh_frame)
	}
}
