use crate::param::KZERO;

#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub fn new(value: u64) -> Self {
        PhysAddr(value)
    }

    pub fn to_virt(&self) -> usize {
        (self.0 as usize).wrapping_add(KZERO)
    }
}
