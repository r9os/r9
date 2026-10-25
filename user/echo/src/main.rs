//! Plan 9's echo (sys/src/cmd/echo.c): print the arguments, separated by
//! spaces and followed by a newline unless the first is -n.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::ffi::CStr;
use echo::{fill, size, split};
use libc::{Errstr, exits, fprint, write};

#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) {
    let mut args = Vec::new();
    if args.try_reserve_exact(argc).is_err() {
        exits("no memory");
    }
    for i in 0..argc {
        // SAFETY: the kernel pushed argc pointers to NUL-terminated strings.
        args.push(unsafe { CStr::from_ptr(*argv.add(i) as *const _) }.to_bytes());
    }
    let (nflag, words) = split(&args);

    let len = size(words);
    let mut buf = Vec::new();
    if buf.try_reserve_exact(len).is_err() {
        exits("no memory");
    }
    buf.resize(len, 0);
    let n = fill(words, nflag, &mut buf);

    if write(1, &buf[..n]) < 0 {
        fprint(2, format_args!("echo: write error: {}\n", Errstr));
        exits("write error");
    }
    exits("");
}
