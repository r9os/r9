use crate::{
    mcslock::{Lock, LockNode},
    mem::{VirtRange, PAGE_SIZE_4K},
    vmem::{Allocator, Arena, Boundary},
};
use alloc::sync::Arc;
use core::alloc::{AllocError, Layout};
use core::ptr::NonNull;

/// VmAlloc is an attempt to write a Bonwick vmem-style allocator.  It currently
/// expects another allocator to exist beforehand.
/// TODO Use the allocator api trait.
pub struct VmemAlloc {
    heap_arena: Arc<Lock<Arena>, &'static dyn core::alloc::Allocator>,
    va_arena: Option<Arc<Lock<Arena>, &'static dyn core::alloc::Allocator>>,
    kmem_default_arena: Option<Arc<Lock<Arena>, &'static dyn core::alloc::Allocator>>,
}

impl VmemAlloc {
    // TODO Specify quantum caching
    pub fn new(
        early_allocator: &'static dyn core::alloc::Allocator,
        heap_range: VirtRange,
    ) -> Self {
        let heap_arena = Arc::new_in(
            Lock::new(
                "heap_arena",
                Arena::new_with_allocator(
                    "heap",
                    Some(Boundary::from(heap_range)),
                    PAGE_SIZE_4K,
                    early_allocator,
                ),
            ),
            early_allocator,
        );

        // va_arena imports from heap_arena, so can use allocations from that heap to
        // allocate blocks of tags.
        let va_arena = Arc::new_in(
            Lock::new(
                "kmem_va",
                Arena::new("kmem_va_arena", None, PAGE_SIZE_4K, Some(heap_arena.clone())),
            ),
            early_allocator,
        );

        // kmem_default_arena - backing store for most object caches
        let kmem_default_arena = Arc::new_in(
            Lock::new(
                "kmem_default_arena",
                Arena::new("kmem_default", None, PAGE_SIZE_4K, Some(va_arena.clone())),
            ),
            early_allocator,
        );

        Self { heap_arena, va_arena: Some(va_arena), kmem_default_arena: Some(kmem_default_arena) }
    }

    /// Create the remaining early arenas.  To be called immediately after new()
    /// as it uses self as the allocator.
    pub fn init(&self) {
        // va_arena imports from heap_arena, so can use allocations from that heap to
        // allocate blocks of tags.
        let va_arena = Arc::new_in(
            Lock::new(
                "kmem_va",
                Arena::new("kmem_va_arena", None, PAGE_SIZE_4K, Some(self.heap_arena.clone())),
            ),
            self,
        );

        // kmem_default_arena - backing store for most object caches
        // let kmem_default_arena = Arc::new_in(
        //     Lock::new(
        //         "kmem_default_arena",
        //         Arena::new("kmem_default", None, PAGE_SIZE_4K, Some(va_arena.clone())),
        //     ),
        //     self,
        // );
        //self.va_arena = Some(va_arena as Allocator);
    }

    pub fn alloc(&self, layout: Layout) -> *mut u8 {
        let node = LockNode::new();
        let mut guard = self
            .kmem_default_arena
            .as_deref()
            .expect("kmem_default_arena not yet created")
            .lock(&node);
        // TODO use layout properly
        guard.alloc(layout.size())
    }
}

unsafe impl core::alloc::Allocator for VmemAlloc {
    fn allocate(
        &self,
        layout: Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, core::alloc::AllocError> {
        let bytes = self.alloc(layout);
        if bytes.is_null() {
            Err(AllocError {})
        } else {
            let nonnull_bytes_ptr = NonNull::new(bytes).unwrap();
            Ok(NonNull::slice_from_raw_parts(nonnull_bytes_ptr, layout.size()))
        }
    }

    unsafe fn deallocate(&self, _ptr: core::ptr::NonNull<u8>, _layout: Layout) {
        todo!()
    }
}

#[cfg(test)]
mod tests {

    use crate::bumpalloc::Bump;

    use super::*;

    #[test]
    fn alloc_with_importing() {
        static BUMP_ALLOC: Bump<{ 4 * PAGE_SIZE_4K }, PAGE_SIZE_4K> = Bump::new(0);
        let vmalloc =
            VmemAlloc::new(&BUMP_ALLOC, VirtRange::with_len(0xffff800000800000, 0x1000000));
        vmalloc.init();
        let b = vmalloc.alloc(unsafe { Layout::from_size_align_unchecked(1024, 1) });
        assert_ne!(b, 0 as *mut u8);
    }
}
