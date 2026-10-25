/* User programs: text at UTZERO, then rodata, then data and bss, each
   segment starting on a page, as the kernel's ELF loader expects.  end is
   the end of the bss, where sbrk starts. */
ENTRY(_start)

PHDRS {
	text PT_LOAD FLAGS(5);		/* R+X */
	rodata PT_LOAD FLAGS(4);	/* R */
	data PT_LOAD FLAGS(6);		/* R+W */
}

SECTIONS {
	. = 0x10000;
	.text : { *(.text*) } :text
	. = ALIGN(4096);
	.rodata : { *(.rodata*) } :rodata
	. = ALIGN(4096);
	.data : { *(.data*) } :data
	.bss : { *(.bss*) *(COMMON) } :data
	PROVIDE(end = .);
	/DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
