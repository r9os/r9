use bitstruct::bitstruct;
use core::fmt;
use num_enum::TryFromPrimitive;

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct EsrEl1(pub u64) {
        iss: u32 = 0 .. 25;
        il: bool = 25;
        ec: u8 = 26..32;
        iss2: u8 = 32 .. 37;
    }
}

impl EsrEl1 {
    /// Try to convert the error into an ExceptionClass enum, or return the original number
    /// as the error.
    pub fn exception_class(&self) -> Result<ExceptionClass, u8> {
        ExceptionClass::try_from(self.ec()).map_err(|e| e.number)
    }
}

impl fmt::Debug for EsrEl1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EsrEl1")
            .field("iss", &format_args!("{:#010x}", self.iss()))
            .field("il", &format_args!("{}", self.il()))
            .field("ec", &format_args!("{:?}", self.exception_class()))
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
