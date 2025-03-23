use core::fmt;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("swtch.S"));

#[derive(Copy, Clone)]
#[repr(C)]
pub struct Context {
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    pub x29: u64, // Frame pointer
    pub x30: u64, // Link register (return address)
    pub sp: u64,
    pub spsr: u64,
}

impl Context {
    pub fn set_return(&mut self, addr: u64) {
        self.x30 = addr;
    }

    pub fn set_stack_pointer(&mut self, addr: u64) {
        self.sp = addr;
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("x19", &format_args!("{:#018x}", self.x19))
            .field("x20", &format_args!("{:#018x}", self.x20))
            .field("x21", &format_args!("{:#018x}", self.x21))
            .field("x22", &format_args!("{:#018x}", self.x22))
            .field("x23", &format_args!("{:#018x}", self.x23))
            .field("x24", &format_args!("{:#018x}", self.x24))
            .field("x25", &format_args!("{:#018x}", self.x25))
            .field("x26", &format_args!("{:#018x}", self.x26))
            .field("x27", &format_args!("{:#018x}", self.x27))
            .field("x28", &format_args!("{:#018x}", self.x28))
            .field("x29", &format_args!("{:#018x}", self.x29))
            .field("x30", &format_args!("{:#018x}", self.x30))
            .field("sp", &format_args!("{:#018x}", self.sp))
            .field("spsr", &format_args!("{:#018x}", self.spsr))
            .finish()
    }
}

unsafe extern "C" {
    pub(crate) fn swtch(from: *mut *mut Context, to: &Context);
}
