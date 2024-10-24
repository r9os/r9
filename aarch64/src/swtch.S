// Context switch
//
//   pub unsafe fn swtch(from: *mut *const Context, to: *const Context)
//
// Save the current registers on the stack, creating
// a Context struct, and save its address in `*from`.
// Switch stacks to `to` and pop previously-saved registers.

.section .text

.globl swtch
swtch:
    // (1) Save callee-saved and other registers onto the caller's stack.
    // We do this so that when we call swtch again (giving this stack pointer
    // as the 'to' address), 
    stp x19, x20, [sp, #-16]!
    stp x21, x22, [sp, #-16]!
    stp x23, x24, [sp, #-16]!
    stp x25, x26, [sp, #-16]!
    stp x27, x28, [sp, #-16]!
    stp x29, x30, [sp, #-16]!
    mov x4, sp
    mrs x5, spsr_el1
    stp x4, x5, [sp, #-16]!

    // (2) Switch stacks.  Once this section completes we will be in the
    // context of the process referred to by the 'to' argument.

    //str x4, [x0]  // Store current SP to first argument (old context)
    mov sp, x1    // Load new SP from second argument (new context)

    // (3) Load the process registers (typically previously saved in (1) during
    // a previous invocation of this subroutine).
    ldp x19, x20, [sp], #16
    ldp x21, x22, [sp], #16
    ldp x23, x24, [sp], #16
    ldp x25, x26, [sp], #16
    ldp x27, x28, [sp], #16
    ldp x29, x30, [sp], #16
    ldp x4, x5, [sp], #16       // Discarding the sp - is that ok?
    msr spsr_el1, x5
    
    // Return to previous location
    ret