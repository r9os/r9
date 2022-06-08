#![feature(asm_const)]
#![feature(asm_sym)]
#![cfg_attr(not(any(test, feature = "cargo-clippy")), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod arch;
mod x86_64;

#[cfg(not(any(test, feature = "cargo-clippy")))]
mod runtime {
    use core::panic::PanicInfo;

    #[panic_handler]
    pub extern "C" fn panic(_info: &PanicInfo) -> ! {
        #[allow(clippy::empty_loop)]
        loop {}
    }
}
