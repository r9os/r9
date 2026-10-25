//! The kernel's ELF loader test.  Its data spans two pages of the file and
//! its bss runs onto another page.  Started with argv ["elftest", "ok"], it
//! checks its arguments, its data, that every word of its bss is zero, and
//! that brk_(0) is where its bss starts, then writes a message from its
//! rodata.  It exits with "elftest" if any check fails.

#![no_std]
#![no_main]

use core::ffi::CStr;
use libc::{brk_, exits, write};

const N: usize = 2 * 4096 / 8 + 1;

const fn pattern() -> [u64; N] {
    let mut words = [0; N];
    let mut i = 0;
    while i < N {
        words[i] = 0x5a5a_0000_0000_0000 | i as u64;
        i += 1;
    }
    words
}

/// Initialised and mutable, so it's in .data, and a word more than two pages.
static mut DATA: [u64; N] = pattern();

/// Zero, so it's in .bss, which runs past the end of the file.
static mut BSS: [u64; 640] = [0; 640];

fn args_ok(argc: usize, argv: *const *const u8) -> bool {
    // SAFETY: the kernel pushed argc pointers to NUL-terminated strings, then
    // a null pointer.
    unsafe {
        let arg = |i: usize| CStr::from_ptr(*argv.add(i) as *const _).to_bytes();
        argc == 2 && arg(0) == b"elftest" && arg(1) == b"ok" && (*argv.add(2)).is_null()
    }
}

fn memory_ok() -> bool {
    let want = pattern();
    // SAFETY: nothing else refers to DATA or BSS.  The reads are volatile so
    // they read memory rather than what the compiler knows was put there.
    let data_ok = (0..N).all(|i| unsafe { (&raw const DATA[i]).read_volatile() } == want[i]);
    let bss_ok = (0..640).all(|i| unsafe { (&raw const BSS[i]).read_volatile() } == 0);
    // The bss starts after the data's file bytes, at or before BSS.
    let bss_start = brk_(0) as usize;
    let brk_ok = bss_start > (&raw const DATA).addr() && bss_start <= (&raw const BSS).addr();
    data_ok && bss_ok && brk_ok
}

#[unsafe(no_mangle)]
pub extern "C" fn main(argc: usize, argv: *const *const u8) {
    if !args_ok(argc, argv) || !memory_ok() {
        exits("elftest");
    }
    let msg = b"hello from elftest\n";
    if write(1, msg) != msg.len() as isize {
        exits("elftest");
    }
    exits("");
}
