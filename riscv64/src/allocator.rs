#[cfg(not(test))]
mod global {
    use core::mem;
    use core::sync::atomic::AtomicPtr;
    use port::allocator::{
        Block, BumpAlloc, QuickFit, global::GlobalHeap, global::GlobalQuickAlloc,
    };

    #[global_allocator]
    static GLOBAL_ALLOCATOR: GlobalQuickAlloc = GlobalQuickAlloc(AtomicPtr::new({
        static mut HEAP: GlobalHeap = GlobalHeap::new();
        static mut ALLOC: QuickFit = QuickFit::new(BumpAlloc::new(unsafe {
            Block::new_from_raw_parts((&raw mut HEAP).cast(), mem::size_of::<GlobalHeap>())
        }));
        &raw mut ALLOC
    }));
}
