#![cfg(not(any(test, feature = "cargo-clippy")))]

extern crate alloc;

use crate::uartmini::MiniUart;
use alloc::alloc::{GlobalAlloc, Layout};
use core::fmt::Write;
use core::panic::PanicInfo;
use port::devcons::PanicConsole;

// TODO
//  - Add qemu integration test
//  - Use LockingConsole via println!() macro once available
//  - Add support for raspi4
#[panic_handler]
pub extern "C" fn panic(info: &PanicInfo) -> ! {
    // Miniuart settings for raspi4 once mapped to higher half
    //let uart = MiniUart::from_addresses(0xffff800000200000, 0xffff800000215000, 0xffff800000215040);

    // Miniuart settings for raspi3 physical memory, as used by qemu currently
    let uart = MiniUart::from_addresses(0x3f200000, 0x3f215000, 0x3f215040);
    uart.init();

    PanicConsole::new(uart).write_fmt(format_args!("{}\n", info)).unwrap();

    // TODO Once the LockingConsole is available, we should use this
    // println!("{}", info);

    #[allow(clippy::empty_loop)]
    loop {}
}

#[alloc_error_handler]
fn oom(_layout: Layout) -> ! {
    panic!("oom");
}

struct FakeAlloc;

unsafe impl GlobalAlloc for FakeAlloc {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        panic!("fake alloc");
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        panic!("fake dealloc");
    }
}

#[global_allocator]
static FAKE_ALLOCATOR: FakeAlloc = FakeAlloc {};
