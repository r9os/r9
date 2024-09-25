use alloc::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use port::{
    mcslock::{Lock, LockNode},
    mem::VirtRange,
    vmemalloc::VmemAlloc,
};

#[cfg(not(test))]
use port::println;

// TODO replace with some sort of OnceLock?  We need this to be dynamically created,
// but we're assuming VmAlloc is Sync.
static VMEM_ALLOC: Lock<Option<&'static mut VmemAlloc>> = Lock::new("vmemalloc", None);

pub fn init(early_allocator: &'static dyn core::alloc::Allocator, heap_range: VirtRange) {
    let node = LockNode::new();
    let mut vmalloc = VMEM_ALLOC.lock(&node);
    *vmalloc = Some({
        static mut MAYBE_VMALLOC: MaybeUninit<VmemAlloc> = MaybeUninit::uninit();
        unsafe {
            MAYBE_VMALLOC.write({
                let vmemalloc = VmemAlloc::new(early_allocator, heap_range);
                vmemalloc.init();
                vmemalloc
            });
            MAYBE_VMALLOC.assume_init_mut()
        }
    });
}

pub struct Allocator {}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        println!("vmalloc::alloc");

        // Get the main allocator
        let node = LockNode::new();
        let mut lock = VMEM_ALLOC.lock(&node);
        let vmemalloc = lock.as_deref_mut().unwrap();
        vmemalloc.alloc(layout)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        panic!("fake dealloc");
    }
}
