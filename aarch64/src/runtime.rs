#![cfg(not(any(test, feature = "cargo-clippy")))]

extern crate alloc;

use crate::registers::rpi_mmio;
use crate::uartmini::MiniUart;
use alloc::alloc::{GlobalAlloc, Layout};
use core::fmt::Write;
use core::panic::PanicInfo;
use port::devcons::PanicConsole;
use port::mem::VirtRange;

// TODO
//  - Add qemu integration test
//  - Use Console via println!() macro once available
//  - Add support for raspi4
#[panic_handler]
pub extern "C" fn panic(info: &PanicInfo) -> ! {
    let mmio = rpi_mmio().expect("mmio base detect failed").to_virt();

    let gpio_range = VirtRange((mmio + 0x200000)..(mmio + 0x20_00b4));
    let aux_range = VirtRange((mmio + 0x215000)..(mmio + 0x21_5008));
    let miniuart_range = VirtRange((mmio + 0x215040)..(mmio + 0x21_5080));

    let uart = MiniUart { gpio_range, aux_range, miniuart_range };
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
