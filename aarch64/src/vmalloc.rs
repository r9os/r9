use alloc::sync::Arc;
use core::{mem::MaybeUninit, ptr::addr_of};
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

static mut EARLY_TAGS_PAGE: [u8; 4096] = [0; 4096];

/// VmAlloc is an attempt to write a Bonwick vmem-style allocator.  It currently
/// expects another allocator to exist beforehand.
/// TODO Use the allocator api trait.
struct VmAlloc {
    heap_arena: Arc<Lock<Arena>>,
    va_arena: Arc<Lock<Arena>>,
}

impl VmAlloc {
    fn new(heap_range: VirtRange) -> Self {
        let early_tags_ptr = addr_of!(EARLY_TAGS_PAGE) as usize;
        let early_tags_size = unsafe { EARLY_TAGS_PAGE.len() };
        let early_tags_range = VirtRange::with_len(early_tags_ptr, early_tags_size);

        let heap_arena = Arc::new(Lock::new(
            "heap_arena",
            Arena::new_with_static_range(
                "heap",
                Some(Boundary::from(heap_range)),
                PAGE_SIZE_4K,
                early_tags_range,
            ),
        ));

        // va_arena imports from heap_arena, so can use allocations from that heap to
        // allocate blocks of tags.
        let va_arena = Arc::new(Lock::new(
            "heap_arena",
            Arena::new("kmem_va", None, PAGE_SIZE_4K, Some(heap_arena.clone())),
        ));

        Self { heap_arena, va_arena }
    }
}

pub fn init(heap_range: VirtRange) {
    let node = LockNode::new();
    let mut vmalloc = VMALLOC.lock(&node);
    *vmalloc = Some({
        static mut MAYBE_VMALLOC: MaybeUninit<VmAlloc> = MaybeUninit::uninit();
        unsafe {
            MAYBE_VMALLOC.write(VmAlloc::new(heap_range));
            MAYBE_VMALLOC.assume_init_mut()
        }
    });
}

pub fn alloc(size: usize) -> *mut u8 {
    let node = LockNode::new();
    let mut lock = VMALLOC.lock(&node);
    let vmalloc = lock.as_deref_mut().unwrap();

    let node = LockNode::new();
    let mut guard = vmalloc.heap_arena.lock(&node);
    guard.alloc(size)
}
