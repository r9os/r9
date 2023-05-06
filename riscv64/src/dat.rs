#[derive(Debug)]
#[repr(align(4096))]
pub struct Mach {
    /// physical id of processor
    pub machno: usize,
    pub online: usize,
}

impl Mach {
    pub fn new() -> Self {
        Self { machno: 0, online: 0 }
    }
}
