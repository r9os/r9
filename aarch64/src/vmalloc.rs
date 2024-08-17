use alloc::sync::Arc;
use core::{alloc::Layout, mem::MaybeUninit};
use port::{
    mcslock::{Lock, LockNode},
    mem::{VirtRange, PAGE_SIZE_4K},
    vmem::{Allocator, Arena, Boundary},
};

// TODO replace with some sort of OnceLock?  We need this to be dynamically created,
// but we're assuming VmAlloc is Sync.
static VMALLOC: Lock<Option<&'static mut VmAlloc>> = Lock::new("vmalloc", None);

// The core arenas are statically allocated.  They cannot be created in const
// functions, so the we declare them as MaybeUninit before intialising and
// referening them from VmAlloc, from where they can be used in the global allocator.
//static mut MAYBE_HEAP_ARENA: MaybeUninit<Arena> = MaybeUninit::uninit();

/// VmAlloc is an attempt to write a Bonwick vmem-style allocator.  It currently
/// expects another allocator to exist beforehand.
/// TODO Use the allocator api trait.
struct VmAlloc {
    heap_arena: Arc<Lock<Arena>, &'static dyn core::alloc::Allocator>,
    _va_arena: Arc<Lock<Arena>, &'static dyn core::alloc::Allocator>,
}

impl VmAlloc {
    fn new(early_allocator: &'static dyn core::alloc::Allocator, heap_range: VirtRange) -> Self {
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
                "kmem_va_arena",
                Arena::new("kmem_va_arena", None, PAGE_SIZE_4K, Some(heap_arena.clone())),
            ),
            early_allocator,
        );

        Self { heap_arena, _va_arena: va_arena }
    }
}

pub fn init(early_allocator: &'static dyn core::alloc::Allocator, heap_range: VirtRange) {
    let node = LockNode::new();
    let mut vmalloc = VMALLOC.lock(&node);
    *vmalloc = Some({
        static mut MAYBE_VMALLOC: MaybeUninit<VmAlloc> = MaybeUninit::uninit();
        unsafe {
            MAYBE_VMALLOC.write(VmAlloc::new(early_allocator, heap_range));
            MAYBE_VMALLOC.assume_init_mut()
        }
    });
}

pub fn alloc(layout: Layout) -> *mut u8 {
    let node = LockNode::new();
    let mut lock = VMALLOC.lock(&node);
    let vmalloc = lock.as_deref_mut().unwrap();

    let node = LockNode::new();
    let mut guard = vmalloc.heap_arena.lock(&node);
    // TODO use layout properly
    guard.alloc(layout.size())
}
