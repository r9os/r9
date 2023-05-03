use crate::{platform::PHYSICAL_MEMORY_OFFSET, runtime::ALLOCATOR};
use alloc::alloc::{alloc, dealloc};
use alloc::alloc::{alloc_zeroed, Layout};
use port::fdt::DeviceTree;

const PAGESIZE: usize = 4096;

/// Convert physical address to virtual address
#[inline]
pub const fn phys_to_virt(paddr: usize) -> usize {
    PHYSICAL_MEMORY_OFFSET + paddr
}

#[inline]
pub fn virt_to_phys(vaddr: usize) -> usize {
    vaddr - PHYSICAL_MEMORY_OFFSET
}

pub fn init_heap(dt: &DeviceTree) {
    extern "C" {
        static end: usize;
    }

    let mut heap_start: usize = 0;
    let mut heap_size: usize = 0;
    let kernel_end = virt_to_phys(unsafe { &end as *const usize as usize });

    for n in dt.nodes() {
        if let Some(name) = DeviceTree::node_name(&dt, &n) {
            if name.contains("memory@") {
                let reg_block_iter = DeviceTree::property_reg_iter(&dt, n);
                for b in reg_block_iter {
                    heap_start = b.addr as usize;
                    heap_size = b.len.unwrap() as usize;
                }
                break;
            }
        }
    }

    heap_size -= kernel_end - heap_start;
    let heap_start: usize = virt_to_phys(unsafe { &end as *const usize as usize });
    unsafe { ALLOCATOR.lock().init(heap_start as *const usize as *mut u8, heap_size) }
}

pub fn kalloc() -> *mut u8 {
    unsafe {
        let layout = Layout::from_size_align(PAGESIZE as usize, 4096).unwrap();
        return alloc_zeroed(layout);
    }
}

pub fn kfree(ptr: *mut u8) {
    unsafe {
        let layout = Layout::from_size_align(PAGESIZE as usize, 4096).unwrap();
        dealloc(ptr, layout);
    }
}
