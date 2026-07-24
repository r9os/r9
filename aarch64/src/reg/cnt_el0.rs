use core::fmt;

use aarch64_cpu::registers::{
    CNTFRQ_EL0, CNTP_CTL_EL0, CNTP_CVAL_EL0, CNTPCT_EL0, Readable, Writeable,
};
use bitstruct::bitstruct;

// CNTPCT_EL0 — 64-bit physical counter
#[derive(Copy, Clone, Default)]
pub struct CntPctEl0(u64);

impl CntPctEl0 {
    pub fn read() -> Self {
        Self(if cfg!(test) { 0 } else { CNTPCT_EL0.extract().into() })
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for CntPctEl0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CntPctEl0").field(&format_args!("{}", self.0)).finish()
    }
}

// CNTFRQ_EL0 — timer frequency in Hz
#[derive(Copy, Clone, Default)]
pub struct CntFrqEl0(u64);

impl CntFrqEl0 {
    pub fn read() -> Self {
        Self(if cfg!(test) { 0 } else { CNTFRQ_EL0.extract().into() })
    }

    pub fn freq(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for CntFrqEl0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CntFrqEl0").field("hz", &format_args!("{}", self.0)).finish()
    }
}

// CNTP_CVAL_EL0 — physical timer compare value (write-only for now)
pub struct CntpCvalEl0;

impl CntpCvalEl0 {
    pub fn write(value: u64) {
        if !cfg!(test) {
            CNTP_CVAL_EL0.set(value);
        }
    }
}

// CNTP_CTL_EL0 — physical timer control register
bitstruct! {
    #[derive(Copy, Clone, Default)]
    pub struct CntpCtlEl0(pub u64) {
        pub enable: bool = 0..1;
        pub imask: bool = 1..2;
        pub istatus: bool = 2..3;
    }
}

impl CntpCtlEl0 {
    pub fn read() -> Self {
        Self(if cfg!(test) { 0 } else { CNTP_CTL_EL0.extract().into() })
    }

    pub fn write(self) {
        if !cfg!(test) {
            CNTP_CTL_EL0.set(self.0);
        }
    }
}

impl fmt::Debug for CntpCtlEl0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CntPctl")
            .field("enable", &self.enable())
            .field("imask", &self.imask())
            .field("istatus", &self.istatus())
            .finish()
    }
}
