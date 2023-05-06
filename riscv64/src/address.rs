use bit_field::BitField;
use core::cmp::{Eq, Ord, Ordering, PartialEq};
use core::convert::From;
use core::convert::Into;
use core::ops::{Add, Sub};

use crate::platform::{PGMASK, PGMASKLEN, PGSHIFT, PGSIZE};

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PhysicalAddress(pub usize);

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct VirtualAddress(pub usize);

pub trait Addr {
    fn as_usize(&self) -> usize;

    fn data_ref(&self) -> &usize;

    fn data_mut(&mut self) -> &mut usize;

    #[inline]
    fn is_page_aligned(&self) -> bool {
        self.as_usize() % PGSIZE == 0
    }

    #[inline]
    fn as_ptr(&self) -> *const u8 {
        self.as_usize() as *const u8
    }

    #[inline]
    fn as_mut_ptr(&self) -> *mut u8 {
        self.as_usize() as *mut u8
    }

    #[inline]
    fn pg_round_up(&mut self) {
        *self.data_mut() = (*self.data_mut() + PGSIZE - 1) & !(PGSIZE - 1)
    }

    #[inline]
    fn pg_round_down(&mut self) {
        *self.data_mut() = *self.data_mut() & !(PGSIZE - 1)
    }

    #[inline]
    fn add_page(&mut self) {
        *self.data_mut() += PGSIZE;
    }
}

impl From<PhysicalAddress> for usize {
    fn from(pa: PhysicalAddress) -> Self {
        pa.0
    }
}

impl From<VirtualAddress> for usize {
    fn from(va: VirtualAddress) -> Self {
        va.0
    }
}

impl Addr for VirtualAddress {
    #[inline]
    fn as_usize(&self) -> usize {
        self.0
    }

    #[inline]
    fn data_ref(&self) -> &usize {
        &self.0
    }

    #[inline]
    fn data_mut(&mut self) -> &mut usize {
        &mut self.0
    }
}

impl Addr for PhysicalAddress {
    #[inline]
    fn as_usize(&self) -> usize {
        self.0
    }

    #[inline]
    fn data_ref(&self) -> &usize {
        &self.0
    }

    #[inline]
    fn data_mut(&mut self) -> &mut usize {
        &mut self.0
    }
}

impl VirtualAddress {
    pub fn new(addr: usize) -> Self {
        Self(addr)
    }

    pub fn compare(&self, other: &Self) -> bool {
        self.0 > other.0
    }

    pub fn equal(&self, other: &Self) -> bool {
        self.0 == other.0
    }

    pub fn add_addr(&self, addr: usize) -> Self {
        Self(self.0 + addr)
    }

    pub fn page_num(&self, level: usize) -> usize {
        (self.0 >> (PGSHIFT + level * PGMASKLEN)) & PGMASK
    }
}

impl PhysicalAddress {
    pub fn new(value: usize) -> Self {
        Self(value)
    }

    pub fn add_addr(&self, addr: usize) -> Self {
        Self(self.0 + addr)
    }

    /// helper to get the address for the page table entry
    pub fn pg_entry(&self) -> usize {
        ((self.0 >> 12) & PGMASK) << 28
            | ((self.0 >> 21) & PGMASK) << 19
            | ((self.0 >> 30) & 0x3ff_ffff) << 10
    }
}

impl Add for VirtualAddress {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for VirtualAddress {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}
