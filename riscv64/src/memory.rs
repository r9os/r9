use crate::{
    dat::Mach,
    platform::{PGSIZE, PHYSICAL_MEMORY_OFFSET},
};
use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use linked_list_allocator::LockedHeap;
use port::fdt::DeviceTree;

extern "C" {
    pub static end: usize; // defined in kernel.ld
}

#[global_allocator]
pub static ALLOCATOR: LockedHeap = LockedHeap::empty();

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
    let mut heap_start: usize = 0;
    let mut heap_size: usize = 0;

    // get the physical end address of the kernel
    let kernel_end =
        virt_to_phys(unsafe { &end as *const usize as usize }) + core::mem::size_of::<Mach>();

    // lookup the memory size
    for n in dt.nodes() {
        if let Some(name) = DeviceTree::node_name(&dt, &n) {
            if name.starts_with("memory@") {
                let reg_block_iter = DeviceTree::property_reg_iter(&dt, n);
                for b in reg_block_iter {
                    heap_start = b.addr as usize;
                    heap_size = b.len.unwrap() as usize;
                }
                break;
            }
        }
    }

    heap_size -= kernel_end - heap_start; // calculate the usable heap size
    let heap_start: usize = kernel_end; // the heap starts where the kernel bss.end section is

    // initalize the allocator
    unsafe { ALLOCATOR.lock().init(heap_start as *const usize as *mut u8, heap_size) }

    port::println!();
    port::println!("heap start: 0x{:X}", heap_start);
    port::println!("heap size : 0x{:X} => {} MiB", heap_size, heap_size / 1024 / 1024);
}

pub fn kalloc() -> *mut u8 {
    unsafe {
        let layout = Layout::from_size_align(PGSIZE as usize, PGSIZE).unwrap();
        return alloc_zeroed(layout);
    }
}

pub fn kfree(ptr: *mut u8) {
    unsafe {
        let layout = Layout::from_size_align(PGSIZE as usize, PGSIZE).unwrap();
        dealloc(ptr, layout);
    }
}
