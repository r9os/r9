use core::fmt;

use aarch64_cpu::registers::{MIDR_EL1, Readable};
use bitstruct::bitstruct;
use num_enum::TryFromPrimitive;

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct MidrEl1(pub u64) {
        revision: u8 = 0..4;
        partnum: u16 = 4..16;
        architecture: u8 = 16..20;
        variant: u8 = 20..24;
        implementer: u16 = 24..32;
    }
}

impl MidrEl1 {
    pub fn read() -> Self {
        Self(if cfg!(test) { 0 } else { MIDR_EL1.extract().into() })
    }

    pub fn partnum_enum(&self) -> Result<PartNum, u16> {
        PartNum::try_from(self.partnum()).map_err(|e| e.number)
    }
}

impl fmt::Debug for MidrEl1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidrEl1")
            .field("revision", &format_args!("{:#x}", self.revision()))
            .field(
                "partnum",
                &format_args!("{:?}", self.partnum_enum().unwrap_or(PartNum::Unknown)),
            )
            .field("architecture", &format_args!("{:#x}", self.architecture()))
            .field("variant", &format_args!("{:#x}", self.variant()))
            .field("implementer", &format_args!("{:#x}", self.implementer()))
            .finish()
    }
}

/// Known IDs for midr_el1's partnum
#[derive(Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u16)]
pub enum PartNum {
    Unknown = 0,
    RaspberryPi1 = 0xb76,
    RaspberryPi2 = 0xc07,
    RaspberryPi3 = 0xd03,
    RaspberryPi4 = 0xd08,
}
