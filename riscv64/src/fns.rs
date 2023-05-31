use crate::{dat::Mach, MACH};

/// get a reference to the boot Mach
pub fn machp() -> &'static mut Mach {
    unsafe { &mut MACH }
}
