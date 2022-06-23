#![cfg(not(any(test, feature = "cargo-clippy")))]

extern crate alloc;

use alloc::alloc::{GlobalAlloc, Layout};
use core::panic::PanicInfo;

#[panic_handler]
pub extern "C" fn panic(_info: &PanicInfo) -> ! {
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
