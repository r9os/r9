//! Plan 9's cat (sys/src/cmd/cat.c): copy each file named, or the standard
//! input if none is, to the standard output.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use core::ffi::CStr;
use core::fmt;
use libc::{Errstr, OREAD, close, exits, fprint, open, read, write};

/// Print "cat: " and the message to fd 2, then exit with the message, as Plan
/// 9's sysfatal does with argv0 set.
fn sysfatal(args: fmt::Arguments) -> ! {
    let msg = format!("{args}");
    fprint(2, format_args!("cat: {msg}\n"));
    exits(&msg)
}

fn cat(f: i32, s: &str) {
    let mut buf = [0u8; 8192];
    let n = loop {
        let n = read(f, &mut buf);
        if n <= 0 {
            break n;
        }
        if write(1, &buf[..n as usize]) != n {
            sysfatal(format_args!("write error copying {s}: {Errstr}"));
        }
    };
    if n < 0 {
        sysfatal(format_args!("error reading {s}: {Errstr}"));
    }
}

#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) {
    if argc == 1 {
        cat(0, "<stdin>");
    } else {
        for i in 1..argc {
            // SAFETY: the kernel pushed argc pointers to NUL-terminated strings.
            let arg = unsafe { CStr::from_ptr(*argv.add(i) as *const _) };
            let name = arg.to_string_lossy();
            let f = open(arg, OREAD);
            if f < 0 {
                sysfatal(format_args!("can't open {name}: {Errstr}"));
            }
            cat(f, &name);
            close(f);
        }
    }
    exits("");
}
