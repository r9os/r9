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

struct VmAlloc {
    heap_arena: Arena,
    //heap_arena: Lock<NonNull<Arena>>,
    // heap_arena: Lock<Arena<'a>>,
    // va_arena: Lock<Arena<'a>>,
}

impl VmAlloc {
    fn new(heap_range: VirtRange) -> Self {
        // Heap_arena is the lowest level arena.  We pass an address range from which
        // it can allocate a block of tags to build the initial structures.
        // let heap_arena = unsafe {
        //     let early_tags_ptr = addr_of!(EARLY_TAGS_PAGE) as usize;
        //     let early_tags_size = EARLY_TAGS_PAGE.len();
        //     let early_tags_range = VirtRange::with_len(early_tags_ptr, early_tags_size);

        //     MAYBE_HEAP_ARENA.write(Arena::new_with_static_range(
        //         "heap",
        //         Some(Boundary::from(heap_range)),
        //         PAGE_SIZE_4K,
        //         early_tags_range,
        //     ));
        //     MAYBE_HEAP_ARENA.assume_init_mut()
        // };

        let early_tags_ptr = addr_of!(EARLY_TAGS_PAGE) as usize;
        let early_tags_size = unsafe { EARLY_TAGS_PAGE.len() };
        let early_tags_range = VirtRange::with_len(early_tags_ptr, early_tags_size);

        let heap_arena = Arena::new_with_static_range(
            "heap",
            Some(Boundary::from(heap_range)),
            PAGE_SIZE_4K,
            early_tags_range,
        );

        // va_arena imports from heap_arena, so can use allocations from that heap to
        // allocate blocks of tags.
        // let va_arena = Arena::new("kmem_va", None, QUANTUM, Some(&heap_arena));

        Self {
            heap_arena,
            //heap_arena,
            //heap_arena: Lock::new(heap_arena.name(), heap_arena),
            //va_arena: Lock::new(va_arena.name(), va_arena),
        }
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

    // let node = LockNode::new();
    // let mut guard = vmalloc.heap_arena.lock(&node);
    //guard.alloc(size)
    vmalloc.heap_arena.alloc(size)
}
