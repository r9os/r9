//! The kernel's EXEC test.  Run with no arguments, it checks that exec of a
//! missing file, a directory, a file without execute permission, a bad
//! image, and too many arguments each fail with Plan 9's error, and return.
//! Then it forks a child, which opens the console twice, once OCEXEC, sets
//! some of its FP/SIMD registers, and execs this program as "x", with the
//! argument "child", and it awaits the child, whose exit must name the file.
//! Last, it execs /bin/echo, so it exits as echo does.  Run as the child, it
//! checks its FP/SIMD registers are zero, and that the OCEXEC descriptor is
//! closed and the other open, and exits with "child".  Anything else exits
//! with what failed.

#![no_std]
#![no_main]

use core::ffi::{CStr, c_char};
use core::mem::MaybeUninit;
use core::ptr;
use libc::{OCEXEC, OWRITE, close, exits, open, rerrstr};
use sys::{AWAIT, EXEC, RFFDG, RFORK, RFPROC, RFREND};

/// Make system call `nr` with two arguments, as libc does.
fn syscall(nr: u64, a0: u64, a1: u64) -> i64 {
    let r: u64;
    // SAFETY: the kernel reads only the arguments it's given, and changes
    // only x0.
    unsafe { core::arch::asm!("svc #0", inlateout("x0") a0 => r, in("x1") a1, in("x8") nr) };
    r as i64
}

/// exec(name, argv), having first set d1, d31 and FPCR's rounding mode, so
/// the program it starts would see them if exec kept them.  It returns only
/// if exec fails.
fn exec_dirty(name: &CStr, argv: &[*const c_char]) -> i64 {
    let r: u64;
    // SAFETY: as syscall's, and the FP/SIMD registers it sets are declared.
    unsafe {
        core::arch::asm!(
            "fmov d1, #1.0",
            "fmov d31, #-2.0",
            "mov {t}, #0x400000",
            "msr fpcr, {t}",
            "svc #0",
            "msr fpcr, xzr",
            t = out(reg) _,
            inlateout("x0") name.as_ptr().addr() as u64 => r,
            in("x1") argv.as_ptr().addr() as u64,
            in("x8") EXEC,
            out("v1") _,
            out("v31") _,
        )
    };
    r as i64
}

/// Whether every FP/SIMD register, FPCR and FPSR are zero.  It's the first
/// thing main does, before the compiler can use any of them.
fn fp_zero() -> bool {
    let mut q = MaybeUninit::<[u128; 32]>::uninit();
    let (fpcr, fpsr): (u64, u64);
    // SAFETY: q is 512 bytes, 16-byte aligned, and writable, and storing the
    // registers changes none of them.
    unsafe {
        core::arch::asm!(
            "stp q0, q1, [{q}, #0]",
            "stp q2, q3, [{q}, #32]",
            "stp q4, q5, [{q}, #64]",
            "stp q6, q7, [{q}, #96]",
            "stp q8, q9, [{q}, #128]",
            "stp q10, q11, [{q}, #160]",
            "stp q12, q13, [{q}, #192]",
            "stp q14, q15, [{q}, #224]",
            "stp q16, q17, [{q}, #256]",
            "stp q18, q19, [{q}, #288]",
            "stp q20, q21, [{q}, #320]",
            "stp q22, q23, [{q}, #352]",
            "stp q24, q25, [{q}, #384]",
            "stp q26, q27, [{q}, #416]",
            "stp q28, q29, [{q}, #448]",
            "stp q30, q31, [{q}, #480]",
            "mrs {fpcr}, fpcr",
            "mrs {fpsr}, fpsr",
            q = in(reg) q.as_mut_ptr(),
            fpcr = out(reg) fpcr,
            fpsr = out(reg) fpsr,
            options(nostack),
        )
    };
    // SAFETY: the asm wrote all of q.
    let q = unsafe { q.assume_init() };
    q.iter().all(|&r| r == 0) && fpcr == 0 && fpsr == 0
}

/// Check that exec of `name` with `argv` fails with `want`.
fn fails(name: &CStr, argv: &[*const c_char], want: &str) {
    if syscall(EXEC, name.as_ptr().addr() as u64, argv.as_ptr().addr() as u64) != -1 {
        exits("exectest: an exec that should fail returned");
    }
    let mut err = [0u8; 128];
    rerrstr(&mut err);
    let n = err.iter().position(|&b| b == 0).unwrap_or(0);
    if &err[..n] != want.as_bytes() {
        exits("exectest: the wrong error");
    }
}

/// Arguments that, with their pointers, are more than a quarter of the
/// 64 KiB stack: a string of 20000 bytes.
static mut BIG: [u8; 20001] = [0; 20001];

fn the_failures() {
    let none = [c"x".as_ptr(), ptr::null()];
    fails(c"/bin/nosuch", &none, "'/bin/nosuch' file does not exist");
    fails(c"/bin", &none, "'/bin' cannot exec directory");
    fails(c"#c/cons", &none, "'#c/cons' permission denied");
    fails(c"/bin/bad", &none, "exec header invalid");
    let big = (&raw mut BIG).cast::<u8>();
    // SAFETY: BIG is 20001 bytes, and nothing else refers to it.
    unsafe { ptr::write_bytes(big, b'a', 20000) };
    fails(c"/bin/echo", &[big.cast(), ptr::null()], "virtual memory allocation failed");
}

/// Fork a child that execs this program with OCEXEC and FP state to lose,
/// and check its exit names the file.
fn the_child() {
    let pid = syscall(RFORK, (RFFDG | RFREND | RFPROC).into(), 0);
    if pid < 0 {
        exits("exectest: rfork");
    }
    if pid == 0 {
        if open(c"#c/cons", OWRITE | OCEXEC) != 3 || open(c"#c/cons", OWRITE) != 4 {
            exits("exectest: open");
        }
        exec_dirty(c"/bin/exectest", &[c"x".as_ptr(), c"child".as_ptr(), ptr::null()]);
        exits("exectest: exec");
    }
    let mut buf = [0u8; 128];
    let n = syscall(AWAIT, buf.as_mut_ptr().addr() as u64, buf.len() as u64);
    let Some(msg) = usize::try_from(n).ok().map(|n| &buf[..n]) else {
        exits("exectest: await");
    };
    // "pid 0 0 0 'exectest pid: child'", quoted for its spaces.
    if !msg.windows(10).any(|w| w == b"'exectest ") || !msg.ends_with(b": child'") {
        exits("exectest: the child's exit");
    }
}

/// Whether this is the child: its one argument is "child".
fn is_child(argc: usize, argv: *const *const u8) -> bool {
    // SAFETY: the kernel pushed argc pointers to NUL-terminated strings.
    argc == 2 && unsafe { CStr::from_ptr(*argv.add(1) as *const c_char) } == c"child"
}

#[unsafe(no_mangle)]
pub extern "C" fn main(argc: usize, argv: *const *const u8) {
    let fp_zero = fp_zero();
    if is_child(argc, argv) {
        if !fp_zero {
            exits("exectest: FP state survived exec");
        }
        if close(3) != -1 || close(4) != 0 {
            exits("exectest: OCEXEC");
        }
        exits("child");
    }
    the_failures();
    the_child();
    let argv = [c"echo".as_ptr(), c"exec".as_ptr(), c"ok".as_ptr(), ptr::null()];
    syscall(EXEC, c"/bin/echo".as_ptr().addr() as u64, argv.as_ptr().addr() as u64);
    exits("exectest: exec of echo");
}
