//! Plan 9's factor (sys/src/cmd/factor.c): print the prime factors of each
//! argument, or with none, of each line of the standard input.

#![no_std]
#![no_main]

use core::ffi::CStr;
use factor::factor;
use libc::{Lines, atof, exits, print, read};

/// Print `n` and its factors, as factor.c's factor.
fn print_factors(n: f64) {
    print!("{n:.0}\n");
    if n > 0.0 && n.is_finite() {
        for f in factor(n) {
            print!("     {f:.0}\n");
        }
        // factor.c prints the blank line in factor, after the factors; 0
        // doesn't get one, which returns before dividing, and a negative or
        // infinite n would loop its wheel forever, where sqrt is NaN or d > s
        // never breaks, so the port stops there.
        print!("\n");
    }
}

#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) {
    if argc > 1 {
        for i in 1..argc {
            // SAFETY: the kernel pushed argc pointers to NUL-terminated strings.
            let arg = unsafe { CStr::from_ptr(*argv.add(i) as *const _) }.to_bytes();
            print_factors(atof(arg));
        }
        exits("");
    }
    // A line at a time, until the end of the file, or a number at most 0.
    let mut lines = Lines::default();
    while let Some(l) = lines.next(|buf| read(0, buf)) {
        let n = atof(l);
        if n <= 0.0 {
            break;
        }
        print_factors(n);
    }
    exits("");
}
