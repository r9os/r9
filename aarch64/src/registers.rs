use bitstruct::bitstruct;
use core::fmt;
use num_enum::TryFromPrimitive;

// GPIO registers
pub const GPFSEL1: u64 = 0x04; // GPIO function select register 1
pub const GPPUD: u64 = 0x94; // GPIO pin pull up/down enable
pub const GPPUDCLK0: u64 = 0x98; // GPIO pin pull up/down enable clock 0

// UART 0 (PL011) registers
pub const UART0_DR: u64 = 0x00; // Data register
pub const UART0_FR: u64 = 0x18; // Flag register
pub const UART0_IBRD: u64 = 0x24; // Integer baud rate divisor
pub const UART0_FBRD: u64 = 0x28; // Fractional baud rate divisor
pub const UART0_LCRH: u64 = 0x2c; // Line control register
pub const UART0_CR: u64 = 0x30; // Control register
pub const UART0_IMSC: u64 = 0x38; // Interrupt mask set clear register
pub const UART0_ICR: u64 = 0x44; // Interrupt clear register

// AUX registers, offset from aux_reg
pub const AUX_ENABLE: u64 = 0x04; // AUX enable register (Mini Uart, SPIs)

// UART1 registers, offset from miniuart_reg
pub const AUX_MU_IO: u64 = 0x00; // AUX IO data register
pub const AUX_MU_IER: u64 = 0x04; // Mini Uart interrupt enable register
pub const AUX_MU_IIR: u64 = 0x08; // Mini Uart interrupt identify register
pub const AUX_MU_LCR: u64 = 0x0c; // Mini Uart line control register
pub const AUX_MU_MCR: u64 = 0x10; // Mini Uart line control register
pub const AUX_MU_LSR: u64 = 0x14; // Mini Uart line status register
pub const AUX_MU_CNTL: u64 = 0x20; // Mini Uart control register
pub const AUX_MU_BAUD: u64 = 0x28; // Mini Uart baudrate register

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
        let mut value: u64 = 0;
        #[cfg(not(test))]
        unsafe {
            core::arch::asm!("mrs {value}, midr_el1", value = out(reg) value);
        }
        Self(value)
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

impl PartNum {
    /// Return the physical MMIO base address for the Raspberry Pi MMIO
    pub fn mmio(&self) -> u64 {
        match self {
            Self::RaspberryPi1 => 0x20000000,
            Self::RaspberryPi2 | Self::RaspberryPi3 => 0x3f000000,
            Self::RaspberryPi4 => 0xfe000000,
            Self::Unknown => 0,
        }
    }
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct EsrEl1(pub u64) {
        iss: u32 = 0..25;
        il: bool = 25;
        ec: u8 = 26..32;
        iss2: u8 = 32..37;
    }
}

impl EsrEl1 {
    /// Try to convert the error into an ExceptionClass enum, or return the original number
    /// as the error.
    pub fn exception_class_enum(&self) -> Result<ExceptionClass, u8> {
        ExceptionClass::try_from(self.ec()).map_err(|e| e.number)
    }
}

impl fmt::Debug for EsrEl1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EsrEl1")
            .field("iss", &format_args!("{:#010x}", self.iss()))
            .field("il", &format_args!("{}", self.il()))
            .field("ec", &format_args!("{:?}", self.exception_class_enum()))
            .field("iss2", &format_args!("{:#04x}", self.iss2()))
            .finish()
    }
}

/// Exception class maps to ESR_EL1 EC bits[31:26]. We skip aarch32 exceptions.
#[derive(Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum ExceptionClass {
    Unknown = 0,
    WaitFor = 1,
    FloatSimd = 7,
    Ls64 = 10,
    BranchTargetException = 13,
    IllegalExecutionState = 14,
    MsrMrsSystem = 24,
    Sve = 25,
    Tstart = 27,
    PointerAuthFailure = 28,
    Sme = 29,
    GranuleProtectionCheck = 30,
    InstructionAbortLowerEl = 32,
    InstructionAbortSameEl = 33,
    PcAlignmentFault = 34,
    DataAbortLowerEl = 36,
    DataAbortSameEl = 37,
    SpAlignmentFault = 38,
    MemoryOperationException = 39,
    TrappedFloatingPointException = 44,
    SError = 47,
    BreakpointLowerEl = 48,
    BreakpointSameEl = 49,
    SoftwareStepLowerEl = 50,
    SoftwareStepSameEl = 51,
    WatchpointLowerEl = 52,
    WatchpointSameEl = 53,
    Brk = 60,
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct EsrEl1IssInstructionAbort(pub u32) {
        ifsc: u8 = 0..6;
        s1ptw: bool = 7;
        ea: bool = 9;
        fnv: bool = 10;
        set: u8 = 11..13;
    }
}

impl EsrEl1IssInstructionAbort {
    pub fn from_esr_el1(r: EsrEl1) -> Option<EsrEl1IssInstructionAbort> {
        r.exception_class_enum()
            .ok()
            .filter(|ec| *ec == ExceptionClass::InstructionAbortSameEl)
            .map(|_| EsrEl1IssInstructionAbort(r.iss()))
    }

    pub fn instruction_fault(&self) -> Result<InstructionFaultStatusCode, u8> {
        InstructionFaultStatusCode::try_from(self.ifsc()).map_err(|e| e.number)
    }
}

#[derive(Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum InstructionFaultStatusCode {
    AddressSizeFaultLevel0 = 0,
    AddressSizeFaultLevel1 = 1,
    AddressSizeFaultLevel2 = 2,
    AddressSizeFaultLevel3 = 3,
    TranslationFaultLevel0 = 4,
    TranslationFaultLevel1 = 5,
    TranslationFaultLevel2 = 6,
    TranslationFaultLevel3 = 7,
    AccessFlagFaultLevel0 = 8,
    AccessFlagFaultLevel1 = 9,
    AccessFlagFaultLevel2 = 10,
    AccessFlagFaultLevel3 = 11,
    PermissionFaultLevel0 = 12,
    PermissionFaultLevel1 = 13,
    PermissionFaultLevel2 = 14,
    PermissionFaultLevel3 = 15,
    SyncExtAbortNotOnWalkOrUpdate = 16,
    SyncExtAbortOnWalkOrUpdateLevelNeg1 = 19,
    SyncExtAbortOnWalkOrUpdateLevel0 = 20,
    SyncExtAbortOnWalkOrUpdateLevel1 = 21,
    SyncExtAbortOnWalkOrUpdateLevel2 = 22,
    SyncExtAbortOnWalkOrUpdateLevel3 = 23,
    SyncParityOrEccErrOnMemAccessNotOnWalk = 24,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevelNeg1 = 27,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel0 = 28,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel1 = 29,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel2 = 30,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel3 = 31,
    GranuleProtectFaultOnWalkOrUpdateLevelNeg1 = 35,
    GranuleProtectFaultOnWalkOrUpdateLevel0 = 36,
    GranuleProtectFaultOnWalkOrUpdateLevel1 = 37,
    GranuleProtectFaultOnWalkOrUpdateLevel2 = 38,
    GranuleProtectFaultOnWalkOrUpdateLevel3 = 39,
    GranuleProtectFaultNotOnWalkOrUpdateLevel = 40,
    AddressSizeFaultLevelNeg1 = 41,
    TranslationFaultLevelNeg1 = 43,
    TlbConflictAbort = 48,
    UnsupportedAtomicHardwareUpdateFault = 49,
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct Vaddr4K4K(pub u64) {
        offset: u16 = 0..12;
        l4idx: u16 = 12..21;
        l3idx: u16 = 21..30;
        l2idx: u16 = 30..39;
        l1idx: u16 = 39..48;
    }
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct Vaddr4K1G(pub u64) {
        offset: u32 = 0..30;
        l2idx: u16 = 30..39;
        l1idx: u16 = 39..48;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // This test is useful for making sense of early-stage exceptions.  Qemu
    // will report an exception of the form below.  Copy the ESR value into
    // this test to break it down.
    //
    // Exception return from AArch64 EL2 to AArch64 EL1 PC 0x8006c
    // Taking exception 3 [Prefetch Abort] on CPU 0
    // ...from EL1 to EL1
    // ...with ESR 0x21/0x86000004
    // ...with FAR 0x80090
    // ...with ELR 0x80090
    // ...to EL1 PC 0x200 PSTATE 0x3c5
    #[test]
    fn test_parse_esr_el1() {
        let r = EsrEl1(0x86000004);
        assert_eq!(r.exception_class_enum().unwrap(), ExceptionClass::InstructionAbortSameEl);
        assert_eq!(
            EsrEl1IssInstructionAbort::from_esr_el1(r).unwrap().instruction_fault().unwrap(),
            InstructionFaultStatusCode::TranslationFaultLevel0
        );
    }

    #[test]
    fn breakdown_vadder() {
        let va = Vaddr4K4K(0xffff_8000_0000_0000);
        assert_eq!(va.l1idx(), 256);
        assert_eq!(va.l2idx(), 0);
        assert_eq!(va.l3idx(), 0);
        assert_eq!(va.l4idx(), 0);
        assert_eq!(va.offset(), 0);

        let va = Vaddr4K4K(0x0000_0000_0008_00a8);
        assert_eq!(va.l1idx(), 0);
        assert_eq!(va.l2idx(), 0);
        assert_eq!(va.l3idx(), 0);
        assert_eq!(va.l4idx(), 128);
        assert_eq!(va.offset(), 168);

        let va = Vaddr4K1G(0xffff_8000_0000_0000);
        assert_eq!(va.l1idx(), 256);
        assert_eq!(va.l2idx(), 0);
        assert_eq!(va.offset(), 0);

        let va = Vaddr4K1G(0x0000_0000_0008_00a8);
        assert_eq!(va.l1idx(), 0);
        assert_eq!(va.l2idx(), 0);
        assert_eq!(va.offset(), 524456);

        let va = Vaddr4K1G(0xffff_8000_0010_00c8);
        assert_eq!(va.l1idx(), 256);
        assert_eq!(va.l2idx(), 0);
        assert_eq!(va.offset(), 0x1000c8);
    }
}
