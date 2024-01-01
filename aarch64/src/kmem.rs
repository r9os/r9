use port::fdt::RegBlock;

use crate::{param::KZERO, vm::Page4K};
use core::{
    fmt,
    iter::{Step, StepBy},
    mem,
    ops::{self, Range},
    slice,
};

// These map to definitions in kernel.ld
extern "C" {
    static etext: [u64; 0];
    static erodata: [u64; 0];
    static ebss: [u64; 0];
    static early_pagetables: [u64; 0];
    static eearly_pagetables: [u64; 0];
}

pub fn text_addr() -> usize {
    0xffff_8000_0000_0000
}

pub fn etext_addr() -> usize {
    unsafe { etext.as_ptr().addr() }
}

pub fn erodata_addr() -> usize {
    unsafe { erodata.as_ptr().addr() }
}

pub fn ebss_addr() -> usize {
    unsafe { ebss.as_ptr().addr() }
}

pub fn early_pagetables_addr() -> usize {
    unsafe { early_pagetables.as_ptr().addr() }
}

pub fn eearly_pagetables_addr() -> usize {
    unsafe { eearly_pagetables.as_ptr().addr() }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
#[repr(transparent)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub const fn new(value: u64) -> Self {
        PhysAddr(value)
    }

    pub const fn addr(&self) -> u64 {
        self.0
    }

    pub const fn to_virt(&self) -> usize {
        (self.0 as usize).wrapping_add(KZERO)
    }

    pub fn from_virt(a: usize) -> Self {
        Self((a - KZERO) as u64)
    }

    pub fn from_ptr<T>(a: *const T) -> Self {
        Self::from_virt(a.addr())
    }

    pub const fn to_ptr_mut<T>(&self) -> *mut T {
        self.to_virt() as *mut T
    }

    pub const fn round_up(&self, step: u64) -> PhysAddr {
        PhysAddr((self.0 + step - 1) & !(step - 1))
    }

    pub const fn round_down(&self, step: u64) -> PhysAddr {
        PhysAddr(self.0 & !(step - 1))
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
    fn steps_between(&startpa: &Self, &endpa: &Self) -> Option<usize> {
        if startpa.0 <= endpa.0 {
            match endpa.0.checked_sub(startpa.0) {
                Some(result) => usize::try_from(result).ok(),
                None => None,
            }
        } else {
            None
        }
    }

    fn forward_checked(startpa: Self, count: usize) -> Option<Self> {
        startpa.0.checked_add(count as u64).map(|x| PhysAddr(x))
    }

    fn backward_checked(startpa: Self, count: usize) -> Option<Self> {
        startpa.0.checked_sub(count as u64).map(|x| PhysAddr(x))
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
    pub fn with_len(start: u64, len: usize) -> Self {
        Self(PhysAddr(start)..PhysAddr(start + len as u64))
    }

    #[allow(dead_code)]
    pub fn offset_addr(&self, offset: u64) -> Option<PhysAddr> {
        let addr = self.0.start + offset;
        if self.0.contains(&addr) {
            Some(addr)
        } else {
            None
        }
    }

    pub fn start(&self) -> PhysAddr {
        self.0.start
    }

    pub fn end(&self) -> PhysAddr {
        self.0.end
    }

    pub fn step_by_rounded(&self, step_size: usize) -> StepBy<Range<PhysAddr>> {
        let startpa = self.start().round_down(step_size as u64);
        let endpa = self.end().round_up(step_size as u64);
        (startpa..endpa).step_by(step_size)
    }
}

impl From<&RegBlock> for PhysRange {
    fn from(r: &RegBlock) -> Self {
        let start = PhysAddr(r.addr);
        let end = start + r.len.unwrap_or(0);
        PhysRange(start..end)
    }
}

unsafe fn page_slice_mut<'a>(pstart: *mut Page4K, pend: *mut Page4K) -> &'a mut [Page4K] {
    let ustart = pstart.addr();
    let uend = pend.addr();
    const PAGE_SIZE: usize = mem::size_of::<Page4K>();
    assert_eq!(ustart % PAGE_SIZE, 0, "page_slice_mut: unaligned start page");
    assert_eq!(uend % PAGE_SIZE, 0, "page_slice_mut: unaligned end page");
    assert!(ustart < uend, "page_slice_mut: bad range");

    let len = (uend - ustart) / PAGE_SIZE;
    unsafe { slice::from_raw_parts_mut(ustart as *mut Page4K, len) }
}

pub fn early_pages() -> &'static mut [Page4K] {
    let early_start = early_pagetables_addr() as *mut Page4K;
    let early_end = eearly_pagetables_addr() as *mut Page4K;
    unsafe { page_slice_mut(early_start, early_end) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm;

    #[test]
    fn physaddr_step() {
        let range = PhysRange(PhysAddr::new(4096)..PhysAddr::new(4096 * 3));
        let pas = range.step_by_rounded(vm::PAGE_SIZE_4K).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(4096), PhysAddr::new(4096 * 2)]);
    }

    #[test]
    fn physaddr_step_rounds_up_and_down() {
        // Start should round down to 8192
        // End should round up to 16384
        let range = PhysRange(PhysAddr::new(9000)..PhysAddr::new(5000 * 3));
        let pas = range.step_by_rounded(vm::PAGE_SIZE_4K).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(4096 * 2), PhysAddr::new(4096 * 3)]);
    }

    #[test]
    fn physaddr_step_2m() {
        let range =
            PhysRange(PhysAddr::new(0x3f000000)..PhysAddr::new(0x3f000000 + 4 * 1024 * 1024));
        let pas = range.step_by_rounded(vm::PAGE_SIZE_2M).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(0x3f000000), PhysAddr::new(0x3f000000 + 2 * 1024 * 1024)]);
    }
}
