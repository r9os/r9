use core::mem::MaybeUninit;

use port::{
    boundarytag::Arena,
    mcslock::{Lock, LockNode},
};

use crate::kmem::heap_virtrange;

static VMALLOC: Lock<Option<&'static mut VmAlloc>> = Lock::new("vmalloc", None);

struct VmAlloc {
    _heap_arena: Arena,
}

impl VmAlloc {
    fn new() -> Self {
        let heap_range = heap_virtrange();
        let quantum = 4096;
        Self {
            _heap_arena: Arena::new_with_static_range(
                "heap",
                heap_range.start(),
                heap_range.size(),
                quantum,
                heap_range,
            ),
        }
    }
}

pub fn init() {
    let node = LockNode::new();
    let mut vmalloc = VMALLOC.lock(&node);
    *vmalloc = Some({
        static mut MAYBE_VMALLOC: MaybeUninit<VmAlloc> = MaybeUninit::uninit();
        unsafe {
            MAYBE_VMALLOC.write(VmAlloc::new());
            MAYBE_VMALLOC.assume_init_mut()
        }
    });
}
