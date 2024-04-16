use crate::platform::PHYSICAL_MEMORY_OFFSET;
use bit_field::BitField;
use bitflags::{bitflags, Flags};
use core::ptr::{read_volatile, write_volatile};
use port::println;

const DEBUG_PHYS_TO_VIRT: bool = false;

const PAGE_TABLE_SIZE: u64 = 4096;

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
pub fn phys_to_virt(paddr: usize) -> usize {
    let vaddr = PHYSICAL_MEMORY_OFFSET + paddr;
    if DEBUG_PHYS_TO_VIRT {
        println!("Physical address {paddr:x} translates to {vaddr:x}");
    }
    vaddr
}

#[derive(Debug)]
pub struct VirtualAddress {
    pub vpn2: SizedInteger<9>,
    pub vpn1: SizedInteger<9>,
    pub vpn0: SizedInteger<9>,
    pub offset: SizedInteger<12>,
}

impl VirtualAddress {
    pub fn get(&self) -> u64 {
        // self.vpn2 << 30 | self.vpn1 << 21 | self.vpn0 << 12 | self.offset
        let mut out = 0u64;
        out.set_bits(30..=38, self.vpn2.into());
        out.set_bits(21..=29, self.vpn1.into());
        out.set_bits(12..=20, self.vpn0.into());
        out.set_bits(0..=11, self.offset.into());
        out
    }
}

impl From<u64> for VirtualAddress {
    fn from(value: u64) -> Self {
        Self {
            vpn2: value.get_bits(30..=38).try_into().unwrap(),
            vpn1: value.get_bits(21..=29).try_into().unwrap(),
            vpn0: value.get_bits(12..=20).try_into().unwrap(),
            offset: value.get_bits(0..=11).try_into().unwrap(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct SizedInteger<const N: usize>(pub u64);

impl<const N: usize> core::fmt::Debug for SizedInteger<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:010x}", self.0)
    }
}

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

bitflags! {
    #[derive(Debug)]
    pub struct PageTableFlags: u8 {
        const D = 1 << 7;
        const A = 1 << 6;
        const G = 1 << 5;
        const U = 1 << 4;
        const X = 1 << 3; // execute
        const W = 1 << 2; // write
        const R = 1 << 1; // read
        const V = 1; // valid
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

    pub fn write_to(&self, addr: u64) {
        unsafe { write_volatile(addr as *mut u64, self.serialize()) }
    }

    pub fn from_paddr(paddr: u64, flags: PageTableFlags) -> PageTableEntry {
        // shifted phyiscal address: pages align to 4k
        let spaddr = paddr >> 12;
        let ppn2 = (spaddr >> 18) & 0x3ff_ffff;
        let ppn1 = (spaddr >> 9) & 0x1ff;
        let ppn0 = spaddr & 0x1ff;
        println!("   {spaddr:x} / {ppn2:x} {ppn1:x} {ppn0:x}");
        PageTableEntry {
            ppn2: ppn2.try_into().unwrap(),
            ppn1: ppn1.try_into().unwrap(),
            ppn0: ppn0.try_into().unwrap(),
            flags,
        }
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

static mut ALLOC_I: u64 = 100;

#[derive(Debug)]
pub struct PageTable {
    addr: u64,
}

impl PageTable {
    const ENTRY_SIZE: u64 = 8;

    pub fn new(addr: u64) -> Self {
        Self { addr }
    }

    pub fn get_paddr(&self) -> u64 {
        self.addr - crate::platform::PHYSICAL_MEMORY_OFFSET as u64
    }

    pub fn get_vaddr(&self) -> u64 {
        self.addr
    }

    pub fn get_entry(&self, at: u16) -> PageTableEntry {
        let addr = self.addr + (at as u64 * Self::ENTRY_SIZE);
        let high = unsafe { read_volatile((addr + 4) as *const u64) };
        let low = unsafe { read_volatile(addr as *const u64) };
        ((high << 32) | low).into()
    }

    pub fn create_entry(&self) {}

    pub fn print_entry(&self, at: u16) {
        println!("  PTE 0x{at:03x} ({at:03})  {:?}", self.get_entry(at));
    }

    pub fn create_pt_at(&self, paddr: u64) -> PageTable {
        let flags = PageTableFlags::W.union(PageTableFlags::R).union(PageTableFlags::V);
        let e = PageTableEntry::from_paddr(paddr, flags);
        let eaddr = self.addr + Self::ENTRY_SIZE * 100;
        println!("  write entry to 0x{eaddr:016x}");
        unsafe { e.write_to(eaddr) };
        PageTable::new(paddr)
    }

    pub fn create_next_level(&self) -> PageTable {
        let e = PageTableEntry::from_paddr(self.get_paddr() + PAGE_TABLE_SIZE, PageTableFlags::V);
        println!("  create next lvl entry {e:?}");
        let eaddr = self.addr + Self::ENTRY_SIZE * 100;
        println!("  write entry to 0x{eaddr:016x}");
        unsafe { e.write_to(eaddr) };
        PageTable::new(self.addr)
    }

    pub fn create_self_ref(&self) -> PageTable {
        let a = self.get_paddr();
        println!("  self ref entry  PT @ paddr {a:08x}  ");
        let flags = PageTableFlags::V.union(PageTableFlags::A);
        // let flags = PageTableFlags::W.union(PageTableFlags::R).union(PageTableFlags::V);
        let e = PageTableEntry::from_paddr(a, flags);
        println!("  create self ref entry {e:?}");
        let eaddr = self.addr + Self::ENTRY_SIZE * 4;
        println!("  write entry to 0x{eaddr:016x}");
        unsafe { e.write_to(eaddr) };
        PageTable::new(self.addr)
    }
}

#[cfg(test)]
mod tests {
    use crate::{PageTableEntry, PageTableFlags};

    #[test]
    fn test_pagetableentry() {
        {
            let entry = PageTableEntry {
                ppn2: 0.try_into().unwrap(),
                ppn1: 1.try_into().unwrap(),
                ppn0: 2.try_into().unwrap(),
                flags: PageTableFlags::W.union(PageTableFlags::R),
            };
            assert_eq!(entry.serialize(), 0b1_000000010_00_00000110);
        }

        {
            let entry = PageTableEntry {
                ppn2: 0x03f0_0000.try_into().unwrap(),
                ppn1: 1.try_into().unwrap(),
                ppn0: 2.try_into().unwrap(),
                flags: PageTableFlags::W.union(PageTableFlags::R),
            };
            assert_eq!(
                entry.serialize(),
                0b11_1111_0000_0000_0000_0000_0000__000000001__000000010__00__00000110
            );
        }
    }
}
