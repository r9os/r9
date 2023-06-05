#![cfg(not(any(test, feature = "cargo-clippy")))]

extern crate alloc;

use crate::registers::MidrEl1;
use crate::uartmini::MiniUart;
use alloc::alloc::{GlobalAlloc, Layout};
use core::fmt::Write;
use core::panic::PanicInfo;
use port::devcons::PanicConsole;

// TODO
//  - Add qemu integration test
//  - Use Console via println!() macro once available
//  - Add support for raspi4
#[panic_handler]
pub extern "C" fn panic(info: &PanicInfo) -> ! {
    const KZERO: u64 = 0xffff800000000000;
    let base = KZERO + MidrEl1::read().partnum_enum().map(|p| p.mmio()).unwrap_or(0);
    let gpio_addr = base + 0x200000;
    let aux_addr = base + 0x215000;
    let miniuart_addr = base + 0x215040;

    let uart = MiniUart::from_addresses(gpio_addr, aux_addr, miniuart_addr);
    //uart.init();

    PanicConsole::new(uart).write_fmt(format_args!("{}\n", info)).unwrap();

    // TODO Once the Console is available, we should use this
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
