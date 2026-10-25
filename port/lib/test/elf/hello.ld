/* Three page-aligned segments at UTZERO: text, rodata, and two pages of data
   followed by a bss that runs onto another page. */
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
	.bss : { *(.bss*) } :data
	/DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
