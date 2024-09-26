use crate::{bumpalloc::Bump, mem::PAGE_SIZE_4K};
use alloc::alloc::{GlobalAlloc, Layout};
use core::{alloc::Allocator, ptr::null_mut};

#[cfg(not(test))]
use crate::println;

static BUMP_ALLOC: Bump<{ 32 * 256 * PAGE_SIZE_4K }, PAGE_SIZE_4K> = Bump::new(0);

pub struct VmAllocator {}

unsafe impl GlobalAlloc for VmAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        println!("vmalloc::alloc");

        let result = BUMP_ALLOC.allocate(layout);
        result.map_or(null_mut(), |b| b.as_ptr() as *mut u8)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        println!("vmalloc::dealloc");
    }
}

pub fn print_status() {
    BUMP_ALLOC.print_status();
}
