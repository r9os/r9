use port::Result;
use port::mem::{PhysRange, VirtRange};

use crate::vm;

/// Map a device register to device memory
/// TODO Maybe make this a macro and wrap the error reporting?
pub fn map_device_register(
    id: &'static str,
    physrange: PhysRange,
    page_size: vm::PageSize,
) -> Result<VirtRange> {
    let page_physrange = physrange.round(page_size.size());

    if let Ok(vr) = vm::kernel_pagetable().map_phys_range(
        id,
        &page_physrange,
        vm::next_free_device_page4k(),
        vm::Entry::rw_device(),
        page_size,
        vm::RootPageTableType::Kernel,
    ) {
        let offset = vr.start() - page_physrange.start().addr() as usize;
        Ok(VirtRange::from_physrange(&physrange, offset))
    } else {
        Err("failed to map device register")
    }
}
