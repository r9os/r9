use crate::registers::EsrEl1;
use port::println;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("trap.S"));

pub fn init() {
    #[cfg(not(test))]
    unsafe {
        // Set up a vector table for any exception that is taken to EL1, then enable IRQ
        core::arch::asm!(
            "adr {tmp}, exception_vectors",
            "msr vbar_el1, {tmp}",
            "msr DAIFClr, #2",
            tmp = out(reg) _,
        );
    }
}

/// Register frame at time interrupt was taken
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct TrapFrame {
    x0: u64,
    x1: u64,
    x2: u64,
    x3: u64,
    x4: u64,
    x5: u64,
    x6: u64,
    x7: u64,
    x8: u64,
    x9: u64,
    x10: u64,
    x11: u64,
    x12: u64,
    x13: u64,
    x14: u64,
    x15: u64,
    x16: u64,
    x17: u64,
    x18: u64,
    x19: u64,
    x20: u64,
    x21: u64,
    x22: u64,
    x23: u64,
    x24: u64,
    x25: u64,
    x26: u64,
    x27: u64,
    x28: u64,
    frame_pointer: u64, // x29
    link_register: u64, // x30
    esr_el1: EsrEl1,
    elr_el1: u64,
    far_el1: u64,
    interrupt_type: u64,
}

#[no_mangle]
pub extern "C" fn trap_unsafe(frame: *mut TrapFrame) {
    unsafe { trap(&mut *frame) }
}

fn trap(frame: &mut TrapFrame) {
    // Just print out the frame and loop for now
    println!("{:x?}", frame);
    loop {}
}
