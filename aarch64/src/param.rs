use port::mem::PAGE_SIZE_4K;

/// Virtual address bits in each half of the address space: ttbr0_el1
/// translates [0, UVA_END) and ttbr1_el1 translates [KVA_BASE, 2^64).
/// TCR_EL1.T0SZ and T1SZ are 64 - VA_BITS.
pub const VA_BITS: usize = 48;

/// End of the range ttbr0_el1 translates.
pub const UVA_END: usize = 1 << VA_BITS;

/// Start of the range ttbr1_el1 translates.
pub const KVA_BASE: usize = !(UVA_END - 1);

// This needs to match KZERO in l.S
pub const KZERO: usize = 0xffff_8000_0000_0000;

const _: () = assert!(KZERO >= KVA_BASE);

/// Maximum number of CPUs (the Raspberry Pi 4 has four cores).
pub const MAXMACH: usize = 4;

/// Base of the kmap region: root entry 510, just below the recursive entry.
/// Each CPU has KMAPSLOTS consecutive 4KiB slots, all in one level 3 table
/// that vminit builds before the MMU is enabled.
pub const KMAPBASE: usize = 0xffff_ff00_0000_0000;

/// Number of kmap slots per CPU: how many pages one CPU can have kmapped at
/// once.
pub const KMAPSLOTS: usize = 8;

// All the slots fit in the one level 3 table that vminit builds.
const _: () = assert!(MAXMACH * KMAPSLOTS <= 512);
const _: () = assert!(KMAPBASE.is_multiple_of(2 << 20));

/// Base of the kernel stack region: the 2MiB after the kmap region.  It holds
/// KSTACKSLOTS slots, each 2 * KSTACKSIZE and aligned to that size: an
/// unmapped guard of KSTACKSIZE, then a stack of KSTACKSIZE above it.  Stacks
/// grow down, so an overflow runs into the guard below the stack.  All the
/// slots are in one level 3 table that vminit builds before the MMU is
/// enabled.
pub const KSTACKBASE: usize = KMAPBASE + (2 << 20);

/// Size of a kernel stack, as STACKSZ in l.S.
pub const KSTACKSIZE: usize = 16 * 1024;

/// Number of kernel stacks.
pub const KSTACKSLOTS: usize = 64;

const _: () = assert!(KSTACKSLOTS * 2 * KSTACKSIZE <= 2 << 20);
const _: () = assert!(KSTACKBASE.is_multiple_of(2 << 20));
const _: () = assert!(KSTACKSIZE.is_multiple_of(PAGE_SIZE_4K));

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
const _: () = assert!(UADDRTOP <= UVA_END);
