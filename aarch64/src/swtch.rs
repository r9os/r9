/// Callee-saved state of a kernel context, as saved and restored by swtch.
/// This is the AAPCS64 callee-saved set (x19-x29, the stack pointer and the
/// return address), minus v8-v15 as the kernel doesn't use FP/SIMD registers.
///
/// A Label isn't Clone: resuming two copies of a saved context would run
/// both on the same stack.
#[derive(Debug)]
#[repr(C)]
pub struct Label {
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
    fp: u64, // x29
    pc: u64, // x30
    sp: u64,
}

impl Label {
    pub const fn new() -> Label {
        Label {
            x19: 0,
            x20: 0,
            x21: 0,
            x22: 0,
            x23: 0,
            x24: 0,
            x25: 0,
            x26: 0,
            x27: 0,
            x28: 0,
            fp: 0,
            pc: 0,
            sp: 0,
        }
    }

    /// A label that, when switched to, starts running `entry` on the stack
    /// whose top is `stack_top` (which must be 16-byte aligned).
    pub fn new_context(entry: extern "C" fn() -> !, stack_top: usize) -> Label {
        debug_assert!(stack_top.is_multiple_of(16), "misaligned stack_top {stack_top:#x}");
        Label { pc: entry as usize as u64, sp: stack_top as u64, ..Label::new() }
    }
}

/// Save the current kernel context in `save`, and resume the one in `next`.
/// Returns when something later switches back to `save`.
///
/// These are raw pointers rather than references because each context stays
/// suspended inside its call to swtch, so references to the same Labels would
/// be live in both contexts at once.
///
/// # Safety
///
/// `save` must be valid for writes, and `next` must have been filled in by a
/// previous swtch, or by Label::new_context with a valid stack.
#[cfg(not(test))]
#[unsafe(naked)]
pub unsafe extern "C" fn swtch(save: *mut Label, next: *const Label) {
    core::arch::naked_asm!(
        "stp x19, x20, [x0, #{x19}]",
        "stp x21, x22, [x0, #{x21}]",
        "stp x23, x24, [x0, #{x23}]",
        "stp x25, x26, [x0, #{x25}]",
        "stp x27, x28, [x0, #{x27}]",
        "stp x29, x30, [x0, #{fp}]",
        "mov x9, sp",
        "str x9, [x0, #{sp}]",
        "ldp x19, x20, [x1, #{x19}]",
        "ldp x21, x22, [x1, #{x21}]",
        "ldp x23, x24, [x1, #{x23}]",
        "ldp x25, x26, [x1, #{x25}]",
        "ldp x27, x28, [x1, #{x27}]",
        "ldp x29, x30, [x1, #{fp}]",
        "ldr x9, [x1, #{sp}]",
        "mov sp, x9",
        "ret",
        x19 = const core::mem::offset_of!(Label, x19),
        x21 = const core::mem::offset_of!(Label, x21),
        x23 = const core::mem::offset_of!(Label, x23),
        x25 = const core::mem::offset_of!(Label, x25),
        x27 = const core::mem::offset_of!(Label, x27),
        fp = const core::mem::offset_of!(Label, fp),
        sp = const core::mem::offset_of!(Label, sp),
    );
}

#[cfg(test)]
pub unsafe extern "C" fn swtch(_save: *mut Label, _next: *const Label) {
    unreachable!()
}
