#![cfg(not(test))]

extern crate alloc;

use crate::kmem::physaddr_as_virt;
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
pub fn panic(info: &PanicInfo) -> ! {
    let mmio = physaddr_as_virt(rpi_mmio().expect("mmio base detect failed").start());

    let gpio_range = VirtRange::with_len(mmio + 0x200000, 0xb4);
    let aux_range = VirtRange::with_len(mmio + 0x215000, 0x8);
    let miniuart_range = VirtRange::with_len(mmio + 0x215040, 0x40);

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
