use crate::fdt::RegBlock;
use core::ops::Range;

pub struct VirtRange(pub Range<usize>);

impl VirtRange {
    pub fn with_len(start: usize, len: usize) -> Self {
        Self(start..start + len)
    }

    pub fn offset_addr(&self, offset: usize) -> Option<usize> {
        let addr = self.0.start + offset;
        if self.0.contains(&addr) {
            Some(addr)
        } else {
            None
        }
    }
}

impl From<&RegBlock> for VirtRange {
    fn from(r: &RegBlock) -> Self {
        let start = r.addr as usize;
        let end = start + r.len.unwrap_or(0) as usize;
        VirtRange(start..end)
    }
}
