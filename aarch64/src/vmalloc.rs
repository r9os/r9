use core::{mem::MaybeUninit, ptr::addr_of};

use port::{
    mcslock::{Lock, LockNode},
    mem::VirtRange,
    vmem::{Arena, Boundary},
};

static VMALLOC: Lock<Option<&'static mut VmAlloc>> = Lock::new("vmalloc", None);

static mut EARLY_TAGS_PAGE: [u8; 4096] = [0; 4096];

struct VmAlloc {
    heap_arena: Arena,
}

impl VmAlloc {
    fn new(heap_range: VirtRange) -> Self {
        let quantum = 4096;

        let early_tags_ptr = unsafe { addr_of!(EARLY_TAGS_PAGE) as usize };
        let early_tags_size = unsafe { EARLY_TAGS_PAGE.len() };
        let early_tags_range = VirtRange::with_len(early_tags_ptr, early_tags_size);

        Self {
            heap_arena: Arena::new_with_static_range(
                "heap",
                Some(Boundary::from(heap_range)),
                quantum,
                None,
                early_tags_range,
            ),
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
    vmalloc.heap_arena.alloc(size)
}
