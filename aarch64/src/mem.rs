use crate::param::KZERO;

#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub fn to_virt(&self) -> usize {
        (self.0 as usize).wrapping_add(KZERO)
    }
}

impl From<usize> for PhysAddr {
    fn from(value: usize) -> Self {
        PhysAddr(value as u64)
    }
}
