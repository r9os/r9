use crate::{fdt::RegBlock, maths};
use core::{
    cmp::{max, min},
    fmt,
    iter::{Step, StepBy},
    ops::{self, Range},
};

pub const PAGE_SIZE_4K: usize = 4 << 10;
pub const PAGE_SIZE_2M: usize = 2 << 20;
pub const PAGE_SIZE_1G: usize = 1 << 30;

pub struct VirtRange(pub Range<usize>);

impl VirtRange {
    pub fn with_len(start: usize, len: usize) -> Self {
        Self(start..start + len)
    }

    pub fn from_physrange(pr: &PhysRange, offset: usize) -> Self {
        Self((pr.0.start.0 as usize + offset)..(pr.0.end.0 as usize + offset))
    }

    pub fn offset_addr(&self, offset: usize) -> Option<usize> {
        let addr = self.0.start + offset;
        if self.0.contains(&addr) { Some(addr) } else { None }
    }

    pub fn start(&self) -> usize {
        self.0.start
    }

    pub fn end(&self) -> usize {
        self.0.end
    }

    pub fn size(&self) -> usize {
        self.0.end - self.0.start
    }
}

impl From<&RegBlock> for VirtRange {
    fn from(r: &RegBlock) -> Self {
        let start = r.addr as usize;
        let end = start + r.len.unwrap_or(0) as usize;
        VirtRange(start..end)
    }
}

impl fmt::Display for VirtRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#018x}..{:#018x}", self.0.start, self.0.end)
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
#[repr(transparent)]
pub struct PhysAddr(pub u64);

impl PhysAddr {
    pub const fn new(value: u64) -> Self {
        PhysAddr(value)
    }

    pub const fn addr(&self) -> u64 {
        self.0
    }

    /// Round up by a power of 2
    pub const fn round_up2(&self, step: u64) -> PhysAddr {
        PhysAddr(maths::round_up2_u64(self.0, step))
    }

    /// Round down by a power of 2
    pub const fn round_down2(&self, step: u64) -> PhysAddr {
        PhysAddr(maths::round_down2_u64(self.0, step))
    }

    pub const fn is_multiple_of(&self, n: u64) -> bool {
        self.0.is_multiple_of(n)
    }
}

impl ops::Add<u64> for PhysAddr {
    type Output = PhysAddr;

    fn add(self, offset: u64) -> PhysAddr {
        PhysAddr(self.0 + offset)
    }
}

/// Note that this implementation will round down the startpa and round up the endpa
impl Step for PhysAddr {
    fn steps_between(&startpa: &Self, &endpa: &Self) -> (usize, Option<usize>) {
        if startpa.0 <= endpa.0 {
            if let Some(diff) = endpa.0.checked_sub(startpa.0) {
                if let Ok(diff) = usize::try_from(diff) {
                    return (diff, Some(diff));
                }
            }
        }
        (0, None)
    }

    fn forward_checked(startpa: Self, count: usize) -> Option<Self> {
        startpa.0.checked_add(count as u64).map(PhysAddr)
    }

    fn backward_checked(startpa: Self, count: usize) -> Option<Self> {
        startpa.0.checked_sub(count as u64).map(PhysAddr)
    }
}

impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysAddr({:#016x})", self.0)?;
        Ok(())
    }
}

pub struct PhysRange(pub Range<PhysAddr>);

impl PhysRange {
    pub fn new(start: PhysAddr, end: PhysAddr) -> Self {
        Self(start..end)
    }

    pub fn with_end(start: u64, end: u64) -> Self {
        Self(PhysAddr(start)..PhysAddr(end))
    }

    pub fn with_len(start: u64, len: usize) -> Self {
        Self(PhysAddr(start)..PhysAddr(start + len as u64))
    }

    pub fn with_pa_len(start: PhysAddr, len: usize) -> Self {
        Self(start..PhysAddr(start.0 + len as u64))
    }

    #[allow(dead_code)]
    pub fn offset_addr(&self, offset: u64) -> Option<PhysAddr> {
        let addr = self.0.start + offset;
        if self.0.contains(&addr) { Some(addr) } else { None }
    }

    pub fn start(&self) -> PhysAddr {
        self.0.start
    }

    pub fn end(&self) -> PhysAddr {
        self.0.end
    }

    pub fn size(&self) -> usize {
        (self.0.end.addr() - self.0.start.addr()) as usize
    }

    pub fn step_by_rounded(&self, step_size: usize) -> StepBy<Range<PhysAddr>> {
        let startpa = self.start().round_down2(step_size as u64);
        let endpa = self.end().round_up2(step_size as u64);
        (startpa..endpa).step_by(step_size)
    }

    pub fn add(&self, other: &PhysRange) -> Self {
        Self(min(self.0.start, other.0.start)..max(self.0.end, other.0.end))
    }

    /// Round extents so that start and end lie on multiples of step_size
    pub fn round(&self, step_size: usize) -> Self {
        Self(self.start().round_down2(step_size as u64)..self.end().round_up2(step_size as u64))
    }
}

impl fmt::Display for PhysRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#018x}..{:#018x}", self.0.start.addr(), self.0.end.addr())
    }
}

impl From<&RegBlock> for PhysRange {
    fn from(r: &RegBlock) -> Self {
        let start = PhysAddr(r.addr);
        let end = start + r.len.unwrap_or(0);
        PhysRange(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physaddr_step() {
        let range = PhysRange(PhysAddr::new(4096)..PhysAddr::new(4096 * 3));
        let pas = range.step_by_rounded(PAGE_SIZE_4K).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(4096), PhysAddr::new(4096 * 2)]);
    }

    #[test]
    fn physaddr_step_rounds_up_and_down() {
        // Start should round down to 8192
        // End should round up to 16384
        let range = PhysRange(PhysAddr::new(9000)..PhysAddr::new(5000 * 3));
        let pas = range.step_by_rounded(PAGE_SIZE_4K).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(4096 * 2), PhysAddr::new(4096 * 3)]);
    }

    #[test]
    fn physaddr_step_2m() {
        let range =
            PhysRange(PhysAddr::new(0x3f000000)..PhysAddr::new(0x3f000000 + 4 * 1024 * 1024));
        let pas = range.step_by_rounded(PAGE_SIZE_2M).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(0x3f000000), PhysAddr::new(0x3f000000 + 2 * 1024 * 1024)]);
    }
}
