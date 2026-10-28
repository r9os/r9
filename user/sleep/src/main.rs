//! Plan 9's sleep (sys/src/cmd/sleep.c): sleep for a number of seconds,
//! whole and fraction, parsed without floating point.

#![no_std]
#![no_main]

use core::ffi::CStr;
use libc::{exits, sleep};
use sleep::parse;

#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) {
    if argc > 1 {
        // SAFETY: the kernel pushed argc pointers to NUL-terminated strings.
        let arg = unsafe { CStr::from_ptr(*argv.add(1) as *const _) }.to_bytes();
        let (mut n, f) = parse(arg);
        while n > 0 {
            sleep(1000);
            n -= 1;
        }
        if let Some(f) = f {
            sleep(f);
        }
    }
    exits("");
}
