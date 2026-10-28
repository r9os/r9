//! Plan 9's factor (sys/src/cmd/factor.c): print the prime factors of each
//! argument.  The stdin path is left out: it reads a line at a time with
//! read and bio, which r9's libc doesn't have yet.  It comes back when
//! there's read.

#![no_std]
#![no_main]

use core::ffi::CStr;
use factor::factor;
use libc::{atof, exits, print};

#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) {
    if argc > 1 {
        for i in 1..argc {
            // SAFETY: the kernel pushed argc pointers to NUL-terminated strings.
            let arg = unsafe { CStr::from_ptr(*argv.add(i) as *const _) }.to_bytes();
            let n = atof(arg);
            print!("{n:.0}\n");
            if n > 0.0 && n.is_finite() {
                for f in factor(n) {
                    print!("     {f:.0}\n");
                }
                // factor.c prints the blank line in factor, after the factors;
                // 0 doesn't get one, which returns before dividing, and a
                // negative or infinite n would loop its wheel forever, where
                // sqrt is NaN or d > s never breaks, so the port stops there.
                print!("\n");
            }
        }
        exits("");
    }
    // factor.c's stdin path would read lines here and stop at a failed read
    // or a number at most 0; the port has only its exit.
    exits("");
}
