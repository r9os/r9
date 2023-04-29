use crate::platform::PHYSICAL_MEMORY_OFFSET;

/// Convert physical address to virtual address
/// See 4.3.2 Virtual Address Translation Process,
/// Volume II: RISC-V Privileged Architectures V20211203 p82
#[inline]
pub const fn phys_to_virt(paddr: usize) -> usize {
    PHYSICAL_MEMORY_OFFSET + paddr
}
