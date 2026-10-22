use port::mem::PAGE_SIZE_4K;

// This needs to match KZERO in l.S
pub const KZERO: usize = 0xffff_8000_0000_0000;

// User address space layout.  User addresses are translated by ttbr0_el1.

/// Start of user text.  The pages below are left unmapped to catch null
/// pointer dereferences.
pub const UTZERO: usize = 0x10000;

/// Top of the user stack.
pub const USTKTOP: usize = 0x0000_8000_0000_0000;

/// Size of the user stack.  It's mapped up front, and nothing is mapped below
/// it, so overflowing it faults.
pub const USTKSIZE: usize = 64 * 1024;

/// End of the user address space.  Above it, root entry 511 recursively maps
/// the user page table.
pub const UADDRTOP: usize = 0x0000_ff80_0000_0000;

const _: () = assert!(UTZERO.is_multiple_of(PAGE_SIZE_4K));
const _: () = assert!(USTKTOP.is_multiple_of(PAGE_SIZE_4K));
const _: () = assert!(USTKSIZE.is_multiple_of(PAGE_SIZE_4K));
const _: () = assert!(UTZERO < USTKTOP - USTKSIZE);
const _: () = assert!(USTKTOP <= UADDRTOP);
