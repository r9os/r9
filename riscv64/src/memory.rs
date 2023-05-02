use crate::platform::PHYSICAL_MEMORY_OFFSET;

/// Convert physical address to virtual address
#[inline]
pub const fn phys_to_virt(paddr: u64) -> u64 {
    PHYSICAL_MEMORY_OFFSET + paddr
}
