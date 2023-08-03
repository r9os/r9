use core::ptr::{read_volatile, write_volatile};
use port::mem::VirtRange;

#[allow(dead_code)]
pub enum GpioPull {
    Off = 0,
    Down,
    Up,
}

/// Delay for count cycles
#[allow(dead_code)]
pub fn delay(count: u32) {
    for _ in 0..count {
        core::hint::spin_loop();
    }
}

/// Write val into the reg RegBlock at offset from reg.addr.
/// Panics if offset is outside any range specified by reg.len.
pub fn write_reg(range: &VirtRange, offset: usize, val: u32) {
    let dst = range.offset_addr(offset).expect("offset outside bounds");
    unsafe { write_volatile(dst as *mut u32, val) }
}

/// Write val|old into the reg RegBlock at offset from reg.addr,
/// where `old` is the existing value.
/// Panics if offset is outside any range specified by reg.len.
#[allow(dead_code)]
pub fn write_or_reg(range: &VirtRange, offset: usize, val: u32) {
    let dst = range.offset_addr(offset).expect("offset outside bounds");
    unsafe {
        let old = read_volatile(dst as *const u32);
        write_volatile(dst as *mut u32, val | old)
    }
}

/// Read from the reg RegBlock at offset from reg.addr.
/// Panics if offset is outside any range specified by reg.len.
pub fn read_reg(range: &VirtRange, offset: usize) -> u32 {
    let src = range.offset_addr(offset).expect("offset outside bounds");
    unsafe { read_volatile(src as *const u32) }
}
