use core::fmt;
use core::mem::{offset_of, size_of};

use crate::reg::esr_el1::EsrEl1;
use port::println;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("trap.S"));

pub fn init() {
    #[cfg(not(test))]
    unsafe {
        // Set up a vector table for any exception that is taken to EL1.  IRQs
        // stay masked until we have interrupt sources to handle.
        core::arch::asm!(
            "adr {tmp}, exception_vectors",
            "msr vbar_el1, {tmp}",
            tmp = out(reg) _,
        );
    }
}

/// Register frame at time interrupt was taken
#[repr(C, align(16))]
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
    sp_el0: u64,
    spsr_el1: u64,
}

// Offsets used by trap.S
const _: () = assert!(offset_of!(TrapFrame, link_register) == 16 * 15);
const _: () = assert!(offset_of!(TrapFrame, elr_el1) == 16 * 16);
const _: () = assert!(offset_of!(TrapFrame, interrupt_type) == 16 * 17);
const _: () = assert!(offset_of!(TrapFrame, sp_el0) == 16 * 17 + 8);
const _: () = assert!(offset_of!(TrapFrame, spsr_el1) == 16 * 18);
const _: () = assert!(size_of::<TrapFrame>() == 16 * 19);

/// SPSR_EL1 for entering EL0: EL0t with debug, SError, IRQ and FIQ masked.
/// Unmask each as it's needed.
const SPSR_EL0T_MASKED: u64 = 0b1111 << 6;

impl TrapFrame {
    /// The exception level the trap was taken from.
    fn exception_level(&self) -> u64 {
        (self.spsr_el1 >> 2) & 0b11
    }
}

impl fmt::Debug for TrapFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrapFrame")
            .field("x0", &format_args!("{:#018x}", self.x0))
            .field("x1", &format_args!("{:#018x}", self.x1))
            .field("x2", &format_args!("{:#018x}", self.x2))
            .field("x3", &format_args!("{:#018x}", self.x3))
            .field("x4", &format_args!("{:#018x}", self.x4))
            .field("x5", &format_args!("{:#018x}", self.x5))
            .field("x6", &format_args!("{:#018x}", self.x6))
            .field("x7", &format_args!("{:#018x}", self.x7))
            .field("x8", &format_args!("{:#018x}", self.x8))
            .field("x9", &format_args!("{:#018x}", self.x9))
            .field("x10", &format_args!("{:#018x}", self.x10))
            .field("x11", &format_args!("{:#018x}", self.x11))
            .field("x12", &format_args!("{:#018x}", self.x12))
            .field("x13", &format_args!("{:#018x}", self.x13))
            .field("x14", &format_args!("{:#018x}", self.x14))
            .field("x15", &format_args!("{:#018x}", self.x15))
            .field("x16", &format_args!("{:#018x}", self.x16))
            .field("x17", &format_args!("{:#018x}", self.x17))
            .field("x18", &format_args!("{:#018x}", self.x18))
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
            .field("x29 (frame_pointer)", &format_args!("{:#018x}", self.frame_pointer))
            .field("x30 (link_register)", &format_args!("{:#018x}", self.link_register))
            .field("esr_el1", &format_args!("{:#?}", self.esr_el1))
            .field("elr_el1", &format_args!("{:#018x}", self.elr_el1))
            .field("far_el1", &format_args!("{:#018x}", self.far_el1))
            .field("interrupt_type", &format_args!("{}", self.interrupt_type))
            .field("sp_el0", &format_args!("{:#018x}", self.sp_el0))
            .field("spsr_el1", &format_args!("{:#018x}", self.spsr_el1))
            .finish()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn trap_unsafe(frame: *mut TrapFrame) {
    unsafe { trap(frame.as_mut().unwrap()) }
}

fn trap(frame: &mut TrapFrame) {
    if frame.esr_el1.ec() == 0x15 {
        syscall(frame);
        return;
    }

    println!("{:#?}", frame);
    panic!("Unhandled exception from EL{}", frame.exception_level());
}

/// Placeholder until there's a real syscall table.  Syscall 3 halts, and any
/// other returns x0 + 1, so that the round trip through EL1 can be checked.
fn syscall(frame: &mut TrapFrame) {
    let syscallid = frame.esr_el1.iss();
    println!("Syscall {syscallid} from EL{}: x0={:#x}", frame.exception_level(), frame.x0);

    if syscallid == 3 {
        println!("Exit");
        loop {
            core::hint::spin_loop();
        }
    }

    frame.x0 += 1;
}

/// Enter EL0 at `entry` with the user stack pointer set to `user_sp`, and the
/// user FP/SIMD state zeroed.
///
/// A TrapFrame is built just below `kstack_top` and restored by trap_return.
/// This leaves SP_EL1 at `kstack_top` while in EL0, so that is the stack
/// exceptions from EL0 will be taken on.
///
/// # Safety
///
/// `kstack_top` must be the 16-byte aligned top of a kernel stack that isn't
/// in use, and `entry` and `user_sp` must be mapped in the current user page
/// table.
pub unsafe fn to_user(kstack_top: usize, entry: u64, user_sp: u64) -> ! {
    let frame = (kstack_top - size_of::<TrapFrame>()) as *mut TrapFrame;
    let mut tf: TrapFrame = unsafe { core::mem::zeroed() };
    tf.elr_el1 = entry;
    tf.sp_el0 = user_sp;
    tf.spsr_el1 = SPSR_EL0T_MASKED;
    unsafe { frame.write(tf) };

    #[cfg(not(test))]
    unsafe {
        // Zero the user FP/SIMD state, so a process can't read whatever the
        // last one left there.  Writing a D register zeroes the rest of its V
        // register, so this clears q0-q31.  The kernel is built softfloat and
        // never uses these registers, so the assembler needs FP enabled here.
        core::arch::asm!(
            ".arch_extension fp",
            "fmov d0, xzr",
            "fmov d1, xzr",
            "fmov d2, xzr",
            "fmov d3, xzr",
            "fmov d4, xzr",
            "fmov d5, xzr",
            "fmov d6, xzr",
            "fmov d7, xzr",
            "fmov d8, xzr",
            "fmov d9, xzr",
            "fmov d10, xzr",
            "fmov d11, xzr",
            "fmov d12, xzr",
            "fmov d13, xzr",
            "fmov d14, xzr",
            "fmov d15, xzr",
            "fmov d16, xzr",
            "fmov d17, xzr",
            "fmov d18, xzr",
            "fmov d19, xzr",
            "fmov d20, xzr",
            "fmov d21, xzr",
            "fmov d22, xzr",
            "fmov d23, xzr",
            "fmov d24, xzr",
            "fmov d25, xzr",
            "fmov d26, xzr",
            "fmov d27, xzr",
            "fmov d28, xzr",
            "fmov d29, xzr",
            "fmov d30, xzr",
            "fmov d31, xzr",
            "msr fpcr, xzr",
            "msr fpsr, xzr",
            "mov sp, {frame}",
            "b trap_return",
            frame = in(reg) frame,
            options(noreturn),
        );
    }

    #[cfg(test)]
    unreachable!()
}
