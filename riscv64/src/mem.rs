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

#[derive(Clone, Copy, Debug)]
struct SizedInteger<const N: usize>(u64);

#[derive(Debug)]
pub struct NumberTooLarge;

impl<const N: usize> TryFrom<u64> for SizedInteger<N> {
    type Error = NumberTooLarge;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if (value.leading_zeros() as usize) < 64 - N {
            return Err(NumberTooLarge);
        }
        Ok(Self(value))
    }
}

impl<const N: usize> From<SizedInteger<N>> for u64 {
    fn from(value: SizedInteger<N>) -> Self {
        value.0
    }
}

#[derive(Debug)]
pub struct PageTableEntry {
    ppn2: SizedInteger<26>,
    ppn1: SizedInteger<9>,
    ppn0: SizedInteger<9>,
    flags: PageTableFlags,
}

impl PageTableEntry {
    pub fn serialize(&self) -> u64 {
        let mut out = 0u64;
        out.set_bits(0..=7, self.flags.bits() as _);
        out.set_bits(10..=18, self.ppn0.into());
        out.set_bits(19..=27, self.ppn1.into());
        out.set_bits(28..=53, self.ppn2.into());
        out
    }

    pub unsafe fn write_to(&self, addr: u64) {
        unsafe { write_volatile(addr as *mut u64, self.serialize()) }
    }
}

impl From<u64> for PageTableEntry {
    fn from(value: u64) -> Self {
        let flags = PageTableFlags::from_bits(value.get_bits(0..=7) as _).unwrap();
        Self {
            ppn2: value.get_bits(28..=53).try_into().unwrap(),
            ppn1: value.get_bits(19..=27).try_into().unwrap(),
            ppn0: value.get_bits(10..=18).try_into().unwrap(),
            flags,
        }
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
