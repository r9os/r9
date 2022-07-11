pub unsafe fn outb(port: u16, b: u8) {
    #[cfg(not(test))]
    unsafe {
        core::arch::asm!("outb %al, %dx", in("dx") port, in("al") b, options(att_syntax));
    }
}

pub unsafe fn outw(port: u16, w: u16) {
    #[cfg(not(test))]
    unsafe {
        core::arch::asm!("outw %ax, %dx", in("dx") port, in("ax") w, options(att_syntax));
    }
}

pub unsafe fn outl(port: u16, l: u32) {
    #[cfg(not(test))]
    unsafe {
        core::arch::asm!("outl %eax, %dx", in("dx") port, in("ax") l, options(att_syntax));
    }
}
