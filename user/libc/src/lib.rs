//! The user library, after Plan 9's libc: the program's entry, system calls,
//! a heap and formatted output.  A program is a `no_main` binary that defines
//! `#[unsafe(no_mangle)] extern "C" fn main(argc: usize, argv: *const *const
//! u8)`, and calls exits.
//!
//! It's only for r9, so on any other target it's empty: a crate that uses it
//! can still build its library, and test it, on the host.

#![cfg(target_os = "none")]
#![no_std]

use core::alloc::{GlobalAlloc, Layout};
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicUsize, Ordering};

use sys::{BRK_, ERRSTR, EXITS, PWRITE};

pub use sys::ERRMAX;

/// Make system call `nr`: svc #0, with the number in x8, the arguments in x0
/// to x3 and the result in x0.  The kernel preserves every other register.
fn syscall(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let r;
    // SAFETY: the kernel only reads the arguments it's given, and only
    // changes x0.
    unsafe {
        core::arch::asm!(
            "svc #0",
            inlateout("x0") a0 => r,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x8") nr,
            options(nostack),
        )
    };
    r
}

unsafe extern "C" {
    fn main(argc: usize, argv: *const *const u8);
}

/// The program's entry.  The kernel starts it like a call, with x0 = argc,
/// x1 = argv and sp aligned, and nothing to return to.  If main returns, the
/// program exits with "main", as Plan 9's _main does.
#[unsafe(no_mangle)]
extern "C" fn _start(argc: usize, argv: *const *const u8) -> ! {
    // SAFETY: the program defines main, and argv is what the kernel pushed.
    unsafe { main(argc, argv) };
    exits("main")
}

/// Exit with `status`: empty means success, as Plan 9's exits(nil).  A status
/// longer than ERRMAX - 1 bytes is truncated.
pub fn exits(status: &str) -> ! {
    let mut buf = [0u8; ERRMAX];
    let p = if status.is_empty() {
        0
    } else {
        let n = status.len().min(ERRMAX - 1);
        buf[..n].copy_from_slice(&status.as_bytes()[..n]);
        buf.as_ptr().addr() as u64
    };
    syscall(EXITS, p, 0, 0, 0);
    // exits doesn't return.
    loop {
        core::hint::spin_loop();
    }
}

/// Write `buf` to `fd` at offset `off`, or at the current offset if it's -1.
/// Return the count written, or -1.
pub fn pwrite(fd: i32, buf: &[u8], off: i64) -> isize {
    syscall(PWRITE, fd as u64, buf.as_ptr().addr() as u64, buf.len() as u64, off as u64) as isize
}

/// Write `buf` to `fd`, as Plan 9's write: pwrite at the current offset.
pub fn write(fd: i32, buf: &[u8]) -> isize {
    pwrite(fd, buf, -1)
}

/// Swap the process's error string with the NUL-terminated string in `buf`,
/// as Plan 9's errstr.  Both are truncated to fit `buf`.  Return 0, or -1.
pub fn errstr(buf: &mut [u8]) -> i32 {
    syscall(ERRSTR, buf.as_mut_ptr().addr() as u64, buf.len() as u64, 0, 0) as i32
}

/// Copy the process's error string into `buf`, NUL-terminated and truncated
/// to fit, leaving it unchanged, as Plan 9's rerrstr: errstr twice.
pub fn rerrstr(buf: &mut [u8]) {
    let mut tmp = [0; ERRMAX];
    errstr(&mut tmp);
    let n = tmp.iter().position(|&b| b == 0).unwrap_or(0).min(buf.len().saturating_sub(1));
    if let Some(dst) = buf.get_mut(..=n) {
        dst[..n].copy_from_slice(&tmp[..n]);
        dst[n] = 0;
    }
    errstr(&mut tmp);
}

/// The process's error string, formatted as Plan 9's %r: it's read with
/// rerrstr, so it's unchanged.
pub struct Errstr;

impl fmt::Display for Errstr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut buf = [0; ERRMAX];
        rerrstr(&mut buf);
        let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
        for chunk in buf[..n].utf8_chunks() {
            f.write_str(chunk.valid())?;
            if !chunk.invalid().is_empty() {
                f.write_str("\u{fffd}")?;
            }
        }
        Ok(())
    }
}

/// Move the end of the bss to `addr`, or return its start if `addr` is 0.
/// Return 0, or -1 if it couldn't be moved.
pub fn brk_(addr: usize) -> isize {
    syscall(BRK_, addr as u64, 0, 0, 0) as isize
}

unsafe extern "C" {
    /// The end of the bss, from the linker script.
    static end: u8;
}

/// The current break, or 0 before brk or sbrk has moved it from end.
static BLOC: AtomicUsize = AtomicUsize::new(0);

fn bloc() -> usize {
    match BLOC.load(Ordering::Relaxed) {
        0 => (&raw const end).addr(),
        b => b,
    }
}

/// Set the break to `p`, rounded up to 8 bytes, as Plan 9's brk.  Return 0, or
/// -1 if it couldn't be set.
pub fn brk(p: usize) -> i32 {
    let bl = p.next_multiple_of(8);
    if brk_(bl) < 0 {
        return -1;
    }
    BLOC.store(bl, Ordering::Relaxed);
    0
}

/// Grow the break by `n` bytes, from the current break rounded up to 8 bytes,
/// as Plan 9's sbrk.  Return the start of the new memory, or None.
pub fn sbrk(n: usize) -> Option<*mut u8> {
    let bl = bloc().next_multiple_of(8);
    if brk_(bl.checked_add(n)?) < 0 {
        return None;
    }
    BLOC.store(bl + n, Ordering::Relaxed);
    Some(bl as *mut u8)
}

/// The heap: memory from sbrk, which is never freed.
struct Brk;

// SAFETY: alloc returns new memory from sbrk, aligned as asked, which no
// other allocation overlaps.
unsafe impl GlobalAlloc for Brk {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let start = bloc().next_multiple_of(8);
        let pad = start.next_multiple_of(layout.align()) - start;
        match sbrk(pad + layout.size()) {
            // SAFETY: the pad bytes are within what sbrk returned.
            Some(p) => unsafe { p.add(pad) },
            None => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static HEAP: Brk = Brk;

/// Formatted output to a file descriptor, through a buffer.
struct Out {
    fd: i32,
    buf: [u8; 256],
    n: usize,
}

impl Out {
    fn flush(&mut self) {
        write(self.fd, &self.buf[..self.n]);
        self.n = 0;
    }
}

impl Write for Out {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if self.n == self.buf.len() {
                self.flush();
            }
            self.buf[self.n] = b;
            self.n += 1;
        }
        Ok(())
    }
}

/// Write `args` to `fd`, as Plan 9's fprint.
pub fn fprint(fd: i32, args: fmt::Arguments) {
    let mut out = Out { fd, buf: [0; 256], n: 0 };
    let _ = out.write_fmt(args);
    out.flush();
}

/// Print to fd 1, as Plan 9's print.
#[macro_export]
macro_rules! print {
    ($($args:tt)*) => {
        $crate::fprint(1, format_args!($($args)*))
    };
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    fprint(2, format_args!("panic: {}\n", info.message()));
    exits("panic")
}
