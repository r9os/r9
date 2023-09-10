use bit_field::BitField;
use bitflags::bitflags;
use core::ptr::{read_volatile, write_volatile};

bitflags! {
  #[derive(Debug)]
  pub struct PageTableFlags: u8 {
      const D = 1 << 7;
      const A = 1 << 6;
      const G = 1 << 5;
      const U = 1 << 4;
      const X = 1 << 3;
      const W = 1 << 2;
      const R = 1 << 1;
      const V = 1;
  }
}

/// Used as an index for PPN and VPN
pub enum PageNumberSegment {
    _0,
    _1,
    _2,
}

#[derive(Debug)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub unsafe fn write_to(&self, addr: u64) {
        unsafe { write_volatile(addr as *mut u64, self.0) }
    }

    pub fn ppn(&self, i: PageNumberSegment) -> u64 {
        use PageNumberSegment::*;
        match i {
            _0 => self.0.get_bits(10..=18),
            _1 => self.0.get_bits(19..=27),
            _2 => self.0.get_bits(28..=53),
        }
    }

    pub fn flags(&self) -> PageTableFlags {
        let bits = self.0.get_bits(0..=7) as u8;
        // safe to unwrap since all bits of a u8 are defined flags
        PageTableFlags::from_bits(bits).unwrap()
    }
}

impl From<u64> for PageTableEntry {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<PageTableEntry> for u64 {
    fn from(value: PageTableEntry) -> Self {
        value.0
    }
}

pub struct PageTable {
    addr: u64,
}

impl PageTable {
    const ENTRY_SIZE: u64 = 8; // 64 bit

    pub fn new(addr: u64) -> Self {
        Self { addr }
    }

    pub fn dump_entry(&self, at: u16) -> PageTableEntry {
        assert!(at < 512, "index out of range: page tables always have 512 entries");
        let addr = self.addr + (at as u64 * Self::ENTRY_SIZE);
        let val = unsafe { read_volatile(addr as *const u64) };
        val.into()
    }
}
