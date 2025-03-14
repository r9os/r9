#![cfg(not(test))]

extern crate alloc;

use alloc::alloc::Layout;
use core::panic::PanicInfo;

#[cfg(not(test))]
use port::println;

// TODO
//  - Add qemu integration test
#[panic_handler]
pub fn panic(info: &PanicInfo) -> ! {
    println!("{}\n", info);

    #[allow(clippy::empty_loop)]
    loop {}
}

#[alloc_error_handler]
fn oom(_layout: Layout) -> ! {
    panic!("oom");
}
