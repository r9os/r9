#![cfg(not(test))]

extern crate alloc;

use alloc::alloc::Layout;
use core::panic::PanicInfo;

#[cfg(not(test))]
use port::iprintln;

#[panic_handler]
pub fn panic(info: &PanicInfo) -> ! {
    iprintln!("{}\n", info);
    crate::shutdown(false)
}

#[alloc_error_handler]
fn oom(_layout: Layout) -> ! {
    panic!("oom");
}
