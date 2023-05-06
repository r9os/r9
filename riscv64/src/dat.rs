#[derive(Debug)]
#[repr(align(4096))]
pub struct Mach {
    /// physical id of processor
    pub machno: usize,
    pub online: bool,
}

impl Mach {
    pub const fn new() -> Self {
        Self { machno: 0, online: false }
    }

    pub fn is_online(&self) -> bool {
        self.online
    }
}
