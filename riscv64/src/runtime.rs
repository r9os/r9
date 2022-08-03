#![cfg(not(any(test, feature = "cargo-clippy")))]

extern crate alloc;

use alloc::alloc::{GlobalAlloc, Layout};
use core::arch::asm;
use core::panic::PanicInfo;

use port::{print, println};

// ///////////////////////////////////
// / LANGUAGE STRUCTURES / FUNCTIONS
// ///////////////////////////////////
#[no_mangle]
extern "C" fn eh_personality() {}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    print!("Panic: ");
    if let Some(p) = info.location() {
        println!("line {}, file {}: {}", p.line(), p.file(), info.message().unwrap());
    } else {
        println!("no information available.");
    }
    abort();
}
#[no_mangle]
extern "C" fn abort() -> ! {
    loop {
        unsafe {
            asm!("wfi");
        }
    }
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
