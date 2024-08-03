#![cfg(not(test))]

extern crate alloc;

use crate::kmem::physaddr_as_virt;
use crate::registers::rpi_mmio;
use crate::uartmini::MiniUart;
use alloc::alloc::{GlobalAlloc, Layout};
use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU8, Ordering::Relaxed};
use num_enum::{FromPrimitive, IntoPrimitive};
use port::bumpalloc::Bump;
use port::devcons::PanicConsole;
use port::mem::{VirtRange, PAGE_SIZE_4K};

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

#[derive(Debug, IntoPrimitive, FromPrimitive)]
#[repr(u8)]
enum AllocatorType {
    #[num_enum(default)]
    None = 0,
    Bump,
}

/// A simple wrapper that allows the allocator to be changed at runtime.
#[repr(C, align(4096))]
struct Allocator {
    bump_alloc: Bump<PAGE_SIZE_4K, PAGE_SIZE_4K>,
    enabled_allocator: AtomicU8,
}

pub fn enable_bump_allocator() {
    ALLOCATOR.enabled_allocator.store(AllocatorType::Bump as u8, Relaxed);
}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match AllocatorType::try_from(self.enabled_allocator.load(Relaxed)) {
            Ok(AllocatorType::None) | Err(_) => panic!("no allocator available for alloc"),
            Ok(AllocatorType::Bump) => unsafe { self.bump_alloc.alloc(layout) },
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        match AllocatorType::try_from(self.enabled_allocator.load(Relaxed)) {
            Ok(AllocatorType::None) | Err(_) => panic!("no allocator available for dealloc"),
            Ok(AllocatorType::Bump) => unsafe { self.bump_alloc.dealloc(ptr, layout) },
        }
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator {
    bump_alloc: Bump::new(0),
    enabled_allocator: AtomicU8::new(AllocatorType::None as u8),
};
