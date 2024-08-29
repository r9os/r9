#![allow(clippy::upper_case_acronyms)]
#![allow(internal_features)]
#![cfg_attr(not(any(test)), no_std)]
#![cfg_attr(not(test), no_main)]
#![feature(allocator_api)]
#![feature(alloc_error_handler)]
#![feature(const_refs_to_static)]
#![feature(core_intrinsics)]
#![feature(strict_provenance)]
#![feature(sync_unsafe_cell)]
#![forbid(unsafe_op_in_unsafe_fn)]

/// Keep this file as sparse as possible for two reasons:
/// 1. We keep the rust main weirdness isolated
/// 2. rust-analyzer gets confused about cfgs and thinks none of this code is
///    enabled and is therefore greyed out in VS Code, so let's move the bulk
///    of the code elsewhere.
mod devcons;
mod init;
mod io;
mod kmem;
mod mailbox;
mod pagealloc;
mod param;
mod registers;
mod runtime;
mod trap;
mod uartmini;
mod uartpl011;
mod vm;
mod vmalloc;

extern crate alloc;

use crate::init::init;

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"));

/// dtb_va is the virtual address of the DTB structure.  The physical address is
/// assumed to be dtb_va-KZERO.
#[no_mangle]
pub extern "C" fn main9(dtb_va: usize) {
    init(dtb_va);
}
