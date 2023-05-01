use crate::platform::PHYSICAL_MEMORY_OFFSET;

/// Convert physical address to virtual address
/// See 4.3.2 Virtual Address Translation Process,
/// Volume II: RISC-V Privileged Architectures V20211203 p82
/// va.off = pa.off
///
/// Physical address:
///         | VPN[2] | VPN[1] | VPN[0] | offset |
///         |[38..30]|[29..21]|[20..12]|[11..0] |
/// Virtual address:
/// |     PPN[2]     | PPN[1] | PPN[0] | offset |
/// |    [55..30]    |[29..21]|[20..12]|[11..0] |
/// NOTE: PPN[2] is 26 bits wide, VPN[2] only 9
#[inline]
pub const fn phys_to_virt(paddr: usize) -> usize {
    PHYSICAL_MEMORY_OFFSET + paddr
}
