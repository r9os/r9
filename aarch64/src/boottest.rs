//! The kernel's boot-time tests: the GIC, before the clock starts; then the
//! clock and timers, kmap, swtch, releasing an address space, and a set of
//! test processes, most of them small assembled programs, which check the
//! syscalls, user memory access and the ELF loader.

use crate::clock;
use crate::gic;
use crate::kmap;
use crate::kstack;
use crate::param::{self, USTKSIZE, USTKTOP, UTZERO};
use crate::proc::{Arm64, enter_user};
use crate::swtch::{self, Label};
use crate::vm;
use aarch64_cpu::registers::{CNTV_CTL_EL0, CNTV_CVAL_EL0, CNTVCT_EL0, Readable, Writeable};
use core::cell::SyncUnsafeCell;
use core::hint::black_box;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;
use port::dat::{MMU, Perm, PutError};
use port::exec::UserRegs;
use port::mem::{PAGE_SIZE_4K, PhysAddr, VirtRange};
use port::println;
use port::segment;
use port::timer::{self, Mode, Timer};

/// Run the GIC's tests, which need the timer's interrupt to themselves, so
/// must run before the clock starts.
pub fn gic() {
    test_timer_interrupt();
    test_unhandled_interrupt();
}

/// Run the other tests, with the clock started and IRQs unmasked, panicking
/// if one fails.
pub fn run() {
    test_clock();
    test_ticks();
    test_one_shot_timer();
    test_kmap();
    test_swtch();
    test_release();
    test_user_processes();
    test_preemption();
    test_fp_state();
    test_sleep_and_wakeup();
    test_sleep_with_another_running();
    test_sleep_alone();
}

// Test programs for test_user_processes.  Each is copied to UTZERO from
// between its start and end labels, so it must be position-independent, and
// starts 8-byte aligned so its literals stay aligned.  The syscall ABI is
// svc #0, with the number in x8, arguments in x0-x5, and the result in x0.
#[cfg(not(test))]
core::arch::global_asm!(
    ".section .rodata.usertest, \"a\"",
    // SYSR1 returns 0, then exits(nil), or exits("sysr1") if it didn't.
    ".balign 8",
    "usertest_sysr1:",
    "   mov x0, #41",
    "   mov x8, #{SYSR1}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 3f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .asciz \"sysr1\"",
    "usertest_sysr1_end:",
    // Count down from 2^29, then exits(nil), without a syscall in between.
    ".balign 8",
    "usertest_spin:",
    "   mov x0, #0x20000000",
    "1: subs x0, x0, #1",
    "   b.ne 1b",
    "   mov x8, #{EXITS}",
    "   svc #0",
    "usertest_spin_end:",
    // Fill every FP/SIMD register with argc, spin for about 300 ms under
    // QEMU, long enough to be preempted several times, then check them all:
    // exits(nil) if each still holds argc, or exits(argv[0]) if any doesn't.
    ".arch_extension simd",
    ".balign 8",
    "usertest_fp:",
    "   mov x2, x0",
    "   mov x5, x1",
    "   .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31",
    "   dup v\\n\\().2d, x2",
    "   .endr",
    "   mov x3, #0x8000000",
    "1: subs x3, x3, #1",
    "   b.ne 1b",
    "   .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31",
    "   umov x4, v\\n\\().d[0]",
    "   cmp x4, x2",
    "   b.ne 2f",
    "   umov x4, v\\n\\().d[1]",
    "   cmp x4, x2",
    "   b.ne 2f",
    "   .endr",
    "   mov x0, #0",
    "   b 3f",
    "2: ldr x0, [x5]",
    "3: mov x8, #{EXITS}",
    "   svc #0",
    "usertest_fp_end:",
    // exits("bye")
    ".balign 8",
    "usertest_bye:",
    "   adr x0, 1f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    "1: .asciz \"bye\"",
    "usertest_bye_end:",
    // An unknown syscall.
    ".balign 8",
    "usertest_badsys:",
    "   mov x8, #9999",
    "   svc #0",
    "usertest_badsys_end:",
    // exits with an unmapped user address, the page after the text.
    ".balign 8",
    "usertest_unmapped:",
    "   ldr x0, 1f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    ".balign 8",
    "1: .quad {UNMAPPED}",
    "usertest_unmapped_end:",
    // exits with a kernel address.
    ".balign 8",
    "usertest_kernel:",
    "   ldr x0, 1f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    ".balign 8",
    "1: .quad {KZERO}",
    "usertest_kernel_end:",
    // Read, then write, the unmapped page after the text.  Each is killed
    // by the fault.
    ".balign 8",
    "usertest_faultread:",
    "   ldr x0, 1f",
    "   ldr x1, [x0]",
    ".balign 8",
    "1: .quad {UNMAPPED}",
    "usertest_faultread_end:",
    ".balign 8",
    "usertest_faultwrite:",
    "   ldr x0, 1f",
    "   str xzr, [x0]",
    ".balign 8",
    "1: .quad {UNMAPPED}",
    "usertest_faultwrite_end:",
    // Write to the text, which is read-only.
    ".balign 8",
    "usertest_textwrite:",
    "   adr x0, usertest_textwrite",
    "   str xzr, [x0]",
    "usertest_textwrite_end:",
    // An undefined instruction.
    ".balign 8",
    "usertest_udf:",
    "   udf #0",
    "usertest_udf_end:",
    // wfi, which SCTLR_EL1.nTWI traps at EL0, but only if it would enter a
    // low-power state (Arm ARM DDI 0487 M.d, D24.2.174): with an interrupt
    // pending, it completes, so try again.
    ".balign 8",
    "usertest_wfi:",
    "1: wfi",
    "   b 1b",
    "usertest_wfi_end:",
    // Read the virtual counter, which CNTKCTL_EL1 traps at EL0.
    ".balign 8",
    "usertest_cntvct:",
    "   mrs x0, cntvct_el0",
    "usertest_cntvct_end:",
    // pwrite(1, "hello\n", 6, -1) returns 6, then exits(nil), or
    // exits("pwrite") if it didn't.
    ".balign 8",
    "usertest_hello:",
    "   mov x0, #1",
    "   adr x1, 3f",
    "   mov x2, #6",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmp x0, #6",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 4f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .ascii \"hello\\n\"",
    "4: .asciz \"pwrite\"",
    "usertest_hello_end:",
    // open(argv[1], OWRITE) gives a new fd, which a pwrite at the -1 offset
    // uses, then a pwrite at an explicit offset of 0: exits(nil) if each does
    // what it should, or exits("open") if any doesn't.
    ".balign 8",
    "usertest_opencons:",
    "   ldr x0, [x1, #8]",
    "   mov x1, #1",
    "   mov x8, #{OPEN}",
    "   svc #0",
    "   cmp x0, #0",
    "   b.lt 1f",
    "   mov x19, x0",
    "   adr x1, 4f",
    "   mov x0, x19",
    "   mov x2, #5",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmp x0, #5",
    "   b.ne 1f",
    "   adr x1, 5f",
    "   mov x0, x19",
    "   mov x2, #6",
    "   mov x3, #0",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmp x0, #6",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 3f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .asciz \"open\"",
    "4: .ascii \"cons\\n\"",
    "5: .ascii \"again\\n\"",
    "usertest_opencons_end:",
    // pwrite to fd 3 returns -1, then exits(nil), or exits("pwrite fd") if it
    // didn't.
    ".balign 8",
    "usertest_badfd:",
    "   mov x0, #3",
    "   adr x1, 4f",
    "   mov x2, #1",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmn x0, #1",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 3f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .asciz \"pwrite fd\"",
    "4: .ascii \"x\"",
    "usertest_badfd_end:",
    // pwrite from an unmapped user address, the page after the text.
    ".balign 8",
    "usertest_badbuf:",
    "   mov x0, #1",
    "   ldr x1, 1f",
    "   mov x2, #1",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    ".balign 8",
    "1: .quad {UNMAPPED}",
    "usertest_badbuf_end:",
    // pwrite to fd 3 fails, and errstr into a buffer on the stack then gives
    // its error, EBADFD: check its first and last characters and its NUL.
    // Then exits(nil), or exits("errstr") if any of that didn't happen.
    ".balign 8",
    "usertest_errstr:",
    "   sub sp, sp, #128",
    "   mov x0, #3",
    "   mov x1, sp",
    "   mov x2, #0",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmn x0, #1",
    "   b.ne 1f",
    "   mov x0, sp",
    "   mov x1, #128",
    "   mov x8, #{ERRSTR}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   ldrb w2, [sp]",
    "   cmp w2, #{EBADFD_FIRST}",
    "   b.ne 1f",
    "   ldrb w2, [sp, #{EBADFD_LEN} - 1]",
    "   cmp w2, #{EBADFD_LAST}",
    "   b.ne 1f",
    "   ldrb w2, [sp, #{EBADFD_LEN}]",
    "   cbnz w2, 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 3f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .asciz \"errstr\"",
    "usertest_errstr_end:",
    // Started with argv ["argv", "ok\n"]: x0 is 2, argv[2] is null, and sp is
    // 16-byte aligned and below argv.  pwrite argv[1], then exits(nil), or
    // exits("argv") if any of that didn't happen.
    ".balign 8",
    "usertest_argv:",
    "   cmp x0, #2",
    "   b.ne 1f",
    "   ldr x2, [x1, #16]",
    "   cbnz x2, 1f",
    "   mov x3, sp",
    "   tst x3, #15",
    "   b.ne 1f",
    "   cmp x3, x1",
    "   b.hi 1f",
    "   ldr x1, [x1, #8]",
    "   mov x0, #1",
    "   mov x2, #3",
    "   mov x3, #-1",
    "   mov x8, #{PWRITE}",
    "   svc #0",
    "   cmp x0, #3",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 3f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "3: .asciz \"argv\"",
    "usertest_argv_end:",
    // brk_(0) returns the bss's start.  brk_ to just past its second page
    // returns 0, and both pages keep what's stored in every word.  Shrinking
    // frees them, and growing again gets pages that read zero in every word,
    // though they were likely the pages just freed.  Then exits(nil), or
    // exits("brk") if any of that didn't happen.
    ".balign 8",
    "usertest_brk:",
    "   ldr x19, 9f",
    "   add x20, x19, #2, lsl #12",
    "   mov x0, #0",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cmp x0, x19",
    "   b.ne 1f",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   mov x1, x19",
    "3: str x19, [x1], #8",
    "   cmp x1, x20",
    "   b.ne 3b",
    "   mov x1, x19",
    "4: ldr x2, [x1], #8",
    "   cmp x2, x19",
    "   b.ne 1f",
    "   cmp x1, x20",
    "   b.ne 4b",
    "   mov x0, x19",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   mov x1, x19",
    "5: ldr x2, [x1], #8",
    "   cbnz x2, 1f",
    "   cmp x1, x20",
    "   b.ne 5b",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 7f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk\"",
    ".balign 8",
    "8: .quad {BSS} + 2 * {PAGE} - 1",
    "9: .quad {BSS}",
    "usertest_brk_end:",
    // Grow the bss by a page and use it, shrink it back, then read the freed
    // page, which kills the process.  exits("brk shrink") if it doesn't.
    ".balign 8",
    "usertest_brkshrink:",
    "   ldr x19, 9f",
    "   add x0, x19, #1",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   str x19, [x19]",
    "   mov x0, x19",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cbnz x0, 1f",
    "   ldr x0, [x19]",
    "1: adr x0, 7f",
    "   mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk shrink\"",
    ".balign 8",
    "9: .quad {BSS}",
    "usertest_brkshrink_end:",
    // brk_ into the guard page below the stack, and past all memory, return
    // -1, then exits(nil), or exits(...) if they didn't.
    ".balign 8",
    "usertest_brkstack:",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cmn x0, #1",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 7f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk stack\"",
    ".balign 8",
    "8: .quad {GUARD}",
    "usertest_brkstack_end:",
    ".balign 8",
    "usertest_brkmem:",
    "   ldr x0, 8f",
    "   mov x8, #{BRK}",
    "   svc #0",
    "   cmn x0, #1",
    "   b.ne 1f",
    "   mov x0, #0",
    "   b 2f",
    "1: adr x0, 7f",
    "2: mov x8, #{EXITS}",
    "   svc #0",
    "7: .asciz \"brk memory\"",
    ".balign 8",
    "8: .quad {HUGE}",
    "usertest_brkmem_end:",
    SYSR1 = const sys::SYSR1,
    EXITS = const sys::EXITS,
    OPEN = const sys::OPEN,
    PWRITE = const sys::PWRITE,
    BRK = const sys::BRK_,
    ERRSTR = const sys::ERRSTR,
    EBADFD_LEN = const port::error::EBADFD.len(),
    EBADFD_FIRST = const port::error::EBADFD.as_bytes()[0],
    EBADFD_LAST = const port::error::EBADFD.as_bytes()[port::error::EBADFD.len() - 1],
    PAGE = const PAGE_SIZE_4K,
    BSS = const BSS,
    GUARD = const USTKTOP - USTKSIZE - PAGE_SIZE_4K + 1,
    HUGE = const BSS + (64 << 30),
    UNMAPPED = const UTZERO + PAGE_SIZE_4K,
    KZERO = const param::KZERO,
);

/// The text of the test program between labels `$start` and `$end`.
macro_rules! usertext {
    ($start:ident, $end:ident) => {{
        unsafe extern "C" {
            static $start: [u8; 0];
            static $end: [u8; 0];
        }
        // SAFETY: the labels bound one program in .rodata.usertest.
        unsafe {
            let start = $start.as_ptr();
            core::slice::from_raw_parts(start, $end.as_ptr().addr() - start.addr())
        }
    }};
}

// The fault tests' statuses spell out these addresses.
const _: () = assert!(UTZERO == 0x10000 && BSS == 0x11000);

/// A test process: its start function, and the status it should exit with.
struct UserTest {
    name: &'static str,
    start: fn(),
    status: &'static [u8],
}

/// Run each test process, checking its exit status, that it freed everything
/// it allocated, and that it reused the first's kernel stack.
fn test_user_processes() {
    let tests = [
        UserTest { name: "sysr1", start: sysr1_start, status: b"" },
        UserTest {
            name: "exits",
            start: || run_user(usertext!(usertest_bye, usertest_bye_end)),
            status: b"bye",
        },
        UserTest {
            name: "bad sys call",
            start: || run_user(usertext!(usertest_badsys, usertest_badsys_end)),
            status: b"sys: bad sys call",
        },
        UserTest {
            name: "unmapped address",
            start: || run_user(usertext!(usertest_unmapped, usertest_unmapped_end)),
            status: b"sys: bad address in syscall",
        },
        UserTest {
            name: "kernel address",
            start: || run_user(usertext!(usertest_kernel, usertest_kernel_end)),
            status: b"sys: bad address in syscall",
        },
        UserTest {
            name: "read an unmapped page",
            start: || run_user(usertext!(usertest_faultread, usertest_faultread_end)),
            status: b"sys: trap: fault read va=0x11000",
        },
        UserTest {
            name: "write an unmapped page",
            start: || run_user(usertext!(usertest_faultwrite, usertest_faultwrite_end)),
            status: b"sys: trap: fault write va=0x11000",
        },
        UserTest {
            name: "write to text",
            start: || run_user(usertext!(usertest_textwrite, usertest_textwrite_end)),
            status: b"sys: trap: fault write va=0x10000",
        },
        UserTest {
            name: "undefined instruction",
            start: || run_user(usertext!(usertest_udf, usertest_udf_end)),
            status: b"sys: trap: undefined instruction pc=0x10000",
        },
        UserTest {
            name: "wfi",
            start: || run_user(usertext!(usertest_wfi, usertest_wfi_end)),
            status: b"sys: trap: wfi or wfe pc=0x10000",
        },
        UserTest {
            name: "read the counter",
            start: || run_user(usertext!(usertest_cntvct, usertest_cntvct_end)),
            status: b"sys: trap: exception class 0x18 pc=0x10000",
        },
        UserTest {
            name: "pwrite",
            start: || run_user(usertext!(usertest_hello, usertest_hello_end)),
            status: b"",
        },
        UserTest {
            name: "open #c/cons",
            start: || {
                run_user_args(
                    usertext!(usertest_opencons, usertest_opencons_end),
                    &[b"open", b"#c/cons"],
                )
            },
            status: b"",
        },
        UserTest {
            name: "open /dev/cons",
            start: || {
                run_user_args(
                    usertext!(usertest_opencons, usertest_opencons_end),
                    &[b"open", b"/dev/cons"],
                )
            },
            status: b"",
        },
        UserTest {
            name: "pwrite to a bad fd",
            start: || run_user(usertext!(usertest_badfd, usertest_badfd_end)),
            status: b"",
        },
        UserTest {
            name: "pwrite from a bad buffer",
            start: || run_user(usertext!(usertest_badbuf, usertest_badbuf_end)),
            status: b"sys: bad address in syscall",
        },
        UserTest {
            name: "errstr",
            start: || run_user(usertext!(usertest_errstr, usertest_errstr_end)),
            status: b"",
        },
        UserTest {
            name: "brk",
            start: || run_user(usertext!(usertest_brk, usertest_brk_end)),
            status: b"",
        },
        UserTest {
            name: "brk shrink",
            start: || run_user(usertext!(usertest_brkshrink, usertest_brkshrink_end)),
            status: b"sys: trap: fault read va=0x11000",
        },
        UserTest {
            name: "brk into the stack",
            start: || run_user(usertext!(usertest_brkstack, usertest_brkstack_end)),
            status: b"",
        },
        UserTest {
            name: "brk past memory",
            start: || run_user(usertext!(usertest_brkmem, usertest_brkmem_end)),
            status: b"",
        },
        UserTest {
            name: "argv",
            start: || {
                run_user_args(usertext!(usertest_argv, usertest_argv_end), &[b"argv", b"ok\n"])
            },
            status: b"",
        },
        UserTest { name: "elf", start: elf_start, status: b"" },
    ];
    let mut first_kstack_top = None;
    for UserTest { name, start, status } in tests {
        let (used_before, _) = port::pagealloc::usage_bytes();
        let p = port::proc::newproc::<Arm64>(start).expect("couldn't create process");
        let kstack_top = p.kstack().top();
        assert_eq!(*first_kstack_top.get_or_insert(kstack_top), kstack_top);
        println!("Executing user process: {name}");
        assert_eq!(port::proc::run(p), status, "{name}");
        assert_eq!(port::pagealloc::usage_bytes().0, used_before, "{name}");
    }
    println!("user process test: ok");
}

/// A process that spins without a syscall for far longer than a quantum is
/// preempted: readied after it, one that exits at once exits first.
fn test_preemption() {
    let (used_before, _) = port::pagealloc::usage_bytes();
    let spin =
        port::proc::newproc::<Arm64>(|| run_user(usertext!(usertest_spin, usertest_spin_end)))
            .expect("couldn't create process");
    let bye = port::proc::newproc::<Arm64>(|| run_user(usertext!(usertest_bye, usertest_bye_end)))
        .expect("couldn't create process");
    let (spin_pid, bye_pid) = (spin.pid, bye.pid);
    let start = now();
    port::proc::ready(spin);
    port::proc::ready(bye);
    let exits = port::proc::schedule::<Arm64>();
    let took = port::clock::to_duration(now() - start, port::clock::frequency().unwrap());
    let order: alloc::vec::Vec<_> = exits.iter().map(|&(pid, _)| pid).collect();
    assert_eq!(order, [bye_pid, spin_pid], "preemption test: wrong exit order");
    assert_eq!(exits[0].1, b"bye", "preemption test");
    assert!(exits[1].1.is_empty(), "preemption test: spin exited badly");
    assert_eq!(port::pagealloc::usage_bytes().0, used_before, "preemption test: leaked");
    println!("preemption test: ok ({took:?})");
}

/// Two processes that each fill their FP/SIMD registers with their own
/// pattern, then spin and are preempted, still find them intact: each exits
/// with its name if it doesn't.
fn test_fp_state() {
    let (used_before, _) = port::pagealloc::usage_bytes();
    let a = port::proc::newproc::<Arm64>(|| {
        run_user_args(usertext!(usertest_fp, usertest_fp_end), &[b"fpa"])
    })
    .expect("couldn't create process");
    let b = port::proc::newproc::<Arm64>(|| {
        run_user_args(usertext!(usertest_fp, usertest_fp_end), &[b"fpb", b"2"])
    })
    .expect("couldn't create process");
    port::proc::ready(a);
    port::proc::ready(b);
    let exits = port::proc::schedule::<Arm64>();
    let lost: alloc::vec::Vec<_> = exits
        .iter()
        .filter(|(_, s)| !s.is_empty())
        .map(|(_, s)| alloc::string::String::from_utf8_lossy(s))
        .collect();
    assert_eq!(exits.len(), 2, "fp state test: {} exits", exits.len());
    assert!(lost.is_empty(), "fp state test: lost by {lost:?}");
    assert_eq!(port::pagealloc::usage_bytes().0, used_before, "fp state test: leaked");
    println!("fp state test: ok");
}

/// A process asleep alone is woken by an interrupt: with nothing else to run,
/// the scheduler idles until a 50 ms timer wakes it.
fn test_sleep_and_wakeup() {
    static R: port::dat::Rendez<Arm64> = port::dat::Rendez::new();
    static WOKEN: AtomicBool = AtomicBool::new(false);
    static WAKER: Timer = Timer::new(Mode::Relative(Duration::from_millis(50)), |_| {
        WOKEN.store(true, Ordering::Relaxed);
        port::proc::wakeup(&R);
    });
    let p = port::proc::newproc::<Arm64>(|| {
        port::proc::sleep(&R, || WOKEN.load(Ordering::Relaxed));
        port::proc::pexit::<Arm64>(b"");
    })
    .expect("couldn't create process");
    let start = now();
    timer::add(&WAKER);
    let status = port::proc::run(p);
    let took = now() - start;
    assert!(status.is_empty(), "sleep test: exited with {status:?}");
    assert!(took >= counts(Duration::from_millis(50)), "sleep test: woke after {took} counts");
    println!("sleep test: ok");
}

/// SLEEP(100) holds the process for at least 100 ms by the counter, while a
/// spinning process runs meanwhile, so the sleep, not the spin, timed it.
fn test_sleep_with_another_running() {
    static HELD: AtomicU64 = AtomicU64::new(0);
    let (used_before, _) = port::pagealloc::usage_bytes();
    let spin =
        port::proc::newproc::<Arm64>(|| run_user(usertext!(usertest_spin, usertest_spin_end)))
            .expect("couldn't create process");
    let sleeper = port::proc::newproc::<Arm64>(|| {
        let start = port::clock::now().expect("no clock");
        let r = port::syscall::syscall::<Arm64>(sys::SLEEP, [100, 0, 0, 0, 0, 0]);
        HELD.store(port::clock::now().expect("no clock") - start, Ordering::Relaxed);
        port::proc::pexit::<Arm64>(if r == 0 { b"" } else { b"sleep(100)" });
    })
    .expect("couldn't create process");
    let sleeper_pid = sleeper.pid;
    port::proc::ready(spin);
    port::proc::ready(sleeper);
    let exits = port::proc::schedule::<Arm64>();
    assert_eq!(exits.len(), 2, "sleep test: {} exits", exits.len());
    let status =
        &exits.iter().find(|(pid, _)| *pid == sleeper_pid).expect("sleep test: sleeper exited").1;
    assert!(status.is_empty(), "sleep test: exited with {status:?}");
    let held = HELD.load(Ordering::Relaxed);
    assert!(
        held >= counts(Duration::from_millis(100)),
        "sleep test: SLEEP(100) held {held} counts"
    );
    assert_eq!(port::pagealloc::usage_bytes().0, used_before, "sleep test: leaked");
    println!("sleep test: ok");
}

/// SLEEP(100) with nothing else ready: the run queue stays empty, so the
/// scheduler idles, and only the sleep's timer ends the idle and wakes the
/// process at its deadline.
fn test_sleep_alone() {
    let (used_before, _) = port::pagealloc::usage_bytes();
    let p = port::proc::newproc::<Arm64>(|| {
        let r = port::syscall::syscall::<Arm64>(sys::SLEEP, [100, 0, 0, 0, 0, 0]);
        port::proc::pexit::<Arm64>(if r == 0 { b"" } else { b"sleep(100)" });
    })
    .expect("couldn't create process");
    let start = now();
    let status = port::proc::run(p);
    let took = now() - start;
    assert!(status.is_empty(), "sleep alone test: exited with {status:?}");
    assert!(
        took >= counts(Duration::from_millis(100)),
        "sleep alone test: woke after {took} counts"
    );
    assert_eq!(port::pagealloc::usage_bytes().0, used_before, "sleep alone test: leaked");
    println!("sleep alone test: ok");
}

/// The first test program's start, which also checks the user access checks
/// against its mappings.
fn sysr1_start() {
    load_user(usertext!(usertest_sysr1, usertest_sysr1_end));
    test_user_access();
    enter_user(UserRegs { pc: UTZERO, sp: USTKTOP, argc: 0, argv: 0 })
}

fn run_user(text: &[u8]) -> ! {
    load_user(text);
    enter_user(UserRegs { pc: UTZERO, sp: USTKTOP, argc: 0, argv: 0 })
}

/// Run `text` with `args` pushed onto its stack.
fn run_user_args(text: &[u8], args: &[&[u8]]) -> ! {
    load_user(text);
    let regs = match port::exec::push_args::<Arm64>(UTZERO, USTKTOP, args) {
        Ok(regs) => regs,
        Err(err) => port::proc::pexit::<Arm64>(err.as_bytes()),
    };
    enter_user(regs)
}

/// The ELF loader's test program, built by xtask (user/elftest).
static ELFTEST: &[u8] = include_bytes!(concat!(env!("R9_USER_BIN"), "/elftest"));

/// Exec ELFTEST with its expected arguments, check that every page of each
/// of its segments has its permissions, and enter it.
fn elf_start() {
    use port::dat::{
        Access::{Read, Write},
        Perm,
    };
    dirty_free_pages();
    let regs = match port::exec::exec::<Arm64>(ELFTEST, &[b"elftest", b"ok"]) {
        Ok(regs) => regs,
        Err(err) => port::proc::pexit::<Arm64>(err.as_bytes()),
    };
    let ok = vm::user_page_ok;
    let elf = port::elf::parse(ELFTEST).expect("elftest didn't parse");
    let mut end = 0;
    for seg in elf.segments() {
        end = (seg.vaddr + seg.memsz).next_multiple_of(PAGE_SIZE_4K);
        for va in (seg.vaddr & !(PAGE_SIZE_4K - 1)..end).step_by(PAGE_SIZE_4K) {
            let writable = seg.perm == Perm::Data;
            assert!(ok(va, Read) && ok(va, Write) == writable, "elftest {:?} at {va:#x}", seg.perm);
        }
    }
    assert!(!ok(end, Read), "above elftest's bss");
    assert!(ok(USTKTOP - 1, Write) && ok(USTKTOP - USTKSIZE, Write), "elftest's stack");
    enter_user(regs)
}

/// The test processes' bss starts, empty, at the page after their text.
const BSS: usize = UTZERO + PAGE_SIZE_4K;

/// Map `text` at UTZERO, and a zeroed stack below USTKTOP, in the current
/// process, and give it an empty bss at BSS.
fn load_user(text: &[u8]) {
    assert!(text.len() <= PAGE_SIZE_4K, "user text too big");
    let text_pa = port::pagealloc::allocate_physpage().expect("couldn't allocate user text");
    {
        let k = kmap::kmap(text_pa);
        // SAFETY: kmaps the whole of the newly allocated page, which nothing
        // else refers to.
        let page = unsafe { core::slice::from_raw_parts_mut(k.va() as *mut u8, PAGE_SIZE_4K) };
        page.fill(0);
        page[..text.len()].copy_from_slice(text);
    }
    Arm64::put(UTZERO, text_pa, Perm::Text).expect("couldn't map user text");
    map_stack();
    segment::set_bss::<Arm64>(VirtRange::new(BSS, BSS));
}

/// Fill some free pages with ones, so a loader that doesn't zero what it maps
/// is caught.  There are enough for every page and page table elftest needs.  The page allocator is next fit, and freeing a page moves its
/// cursor there (BitmapPageAlloc::deallocate), so freeing these last first
/// makes them the next it hands out.  That's checked here, so the test can't
/// silently stop working if the allocator changes.
fn dirty_free_pages() {
    let mut pages = [PhysAddr::new(0); 32];
    for pa in &mut pages {
        *pa = port::pagealloc::allocate_physpage().expect("couldn't allocate a page");
        let k = kmap::kmap(*pa);
        // SAFETY: kmaps the whole of the newly allocated page.
        unsafe { core::ptr::write_bytes(k.va() as *mut u8, 0xff, PAGE_SIZE_4K) };
    }
    for pa in pages.into_iter().rev() {
        port::pagealloc::free_physpage(pa).expect("couldn't free a page");
    }
    let next = port::pagealloc::allocate_physpage().expect("couldn't allocate a page");
    assert_eq!(next, pages[0], "the next page allocated isn't a dirtied one");
    port::pagealloc::free_physpage(next).expect("couldn't free a page");
}

/// Map a zeroed stack below USTKTOP in the current process.
fn map_stack() {
    let stack_range = VirtRange::new(USTKTOP - USTKSIZE, USTKTOP);
    segment::map_zeroed::<Arm64>(stack_range).expect("couldn't allocate user stack");
}

/// Check user_page_ok against the test process's mappings, which are
/// active: text is read-only, the stack is read-write, and everything else,
/// including the kernel and the recursive window, is inaccessible.
fn test_user_access() {
    use port::dat::Access::{Read, Write};
    let ok = vm::user_page_ok;
    assert!(ok(UTZERO, Read) && !ok(UTZERO, Write), "user text");
    assert!(ok(USTKTOP - 1, Read) && ok(USTKTOP - USTKSIZE, Write), "user stack");
    assert!(!ok(USTKTOP - USTKSIZE - 1, Read), "below the user stack");
    assert!(!ok(0, Read), "null");
    assert!(!ok(param::UADDRTOP, Read), "user recursive window");
    assert!(!ok(param::KZERO, Read), "kernel");
    println!("user access test: ok");
}

/// The counter is running, and the clock never goes backwards.
fn test_clock() {
    let first = now();
    let mut last = first;
    for _ in 0..100_000 {
        let t = now();
        assert!(t >= last, "clock test: went backwards from {last} to {t}");
        last = t;
    }
    assert!(last > first, "clock test: the counter isn't running");
}

/// The virtual timer's interrupt is taken once by its handler, which
/// disables the timer, and is ended: no longer active at the GIC.
fn test_timer_interrupt() {
    static TAKEN: AtomicUsize = AtomicUsize::new(0);
    fn handler() {
        disarm_timer();
        TAKEN.fetch_add(1, Ordering::Relaxed);
    }
    let intid = clock::vtimer_intid();
    gic::attach(intid, handler);
    arm_timer();
    take_irqs_for(Duration::from_millis(10));
    gic::detach(intid);
    let taken = TAKEN.load(Ordering::Relaxed);
    assert_eq!(taken, 1, "timer interrupt test: taken {taken} times");
    assert!(!gic::active(intid), "timer interrupt test: not ended");
}

/// The virtual timer's interrupt, with no handler, is disabled when taken.
fn test_unhandled_interrupt() {
    let intid = clock::vtimer_intid();
    gic::enable(intid);
    arm_timer();
    take_irqs_for(Duration::from_millis(10));
    disarm_timer();
    assert!(!gic::enabled(intid), "unhandled interrupt test: still enabled");
    assert!(!gic::active(intid), "unhandled interrupt test: not ended");
}

/// Arm the virtual timer to fire now: its condition holds once the counter
/// reaches CNTV_CVAL_EL0 (Arm ARM DDI 0487 M.d, D24.10.29).
fn arm_timer() {
    CNTV_CVAL_EL0.set(CNTVCT_EL0.get());
    CNTV_CTL_EL0.set(1);
}

/// Disable the virtual timer.  Its interrupt is level-triggered, so the isb
/// makes sure it has dropped before the GIC's EOI, as Linux's
/// arch_timer_reg_write does.
fn disarm_timer() {
    CNTV_CTL_EL0.set(0);
    aarch64_cpu::asm::barrier::isb(aarch64_cpu::asm::barrier::SY);
}

/// Unmask IRQs at EL1 for `d`, then mask them again.
fn take_irqs_for(d: Duration) {
    // SAFETY: unmasking IRQs lets the GIC's enabled interrupts in, and only
    // the test's is enabled.  Neither asm! says it leaves memory alone, so
    // each is a compiler barrier.
    unsafe { core::arch::asm!("msr daifclr, #2", options(nostack)) };
    wait(d);
    unsafe { core::arch::asm!("msr daifset, #2", options(nostack)) };
}

/// The counter, now.
fn now() -> u64 {
    port::clock::now().expect("no clock")
}

/// `d` in counts.
fn counts(d: Duration) -> u64 {
    port::clock::to_counts(d, port::clock::frequency().expect("no clock"))
}

/// Spin for `d`, by the counter.
fn wait(d: Duration) {
    let end = now() + counts(d);
    while now() < end {
        core::hint::spin_loop();
    }
}

/// The HZ clock ticks at HZ: over 100 ms by the counter, ticks advances by
/// 10, give or take 2 for the ticks either side.
fn test_ticks() {
    let first = timer::ticks();
    wait(Duration::from_millis(100));
    let ticks = timer::ticks() - first;
    assert!((8..=12).contains(&ticks), "ticks test: {ticks} ticks in 100 ms");
}

/// A one-shot timer fires once, not before its deadline, and soon after it.
fn test_one_shot_timer() {
    static FIRED: AtomicUsize = AtomicUsize::new(0);
    static AT: AtomicU64 = AtomicU64::new(0);
    static ONE_SHOT: Timer = Timer::new(Mode::Relative(Duration::from_millis(20)), |_| {
        FIRED.fetch_add(1, Ordering::Relaxed);
        AT.store(now(), Ordering::Relaxed);
    });
    let deadline = now() + counts(Duration::from_millis(20));
    timer::add(&ONE_SHOT);
    wait(Duration::from_millis(60));
    let fired = FIRED.load(Ordering::Relaxed);
    assert_eq!(fired, 1, "one-shot timer test: fired {fired} times");
    let late = AT.load(Ordering::Relaxed) as i64 - deadline as i64;
    let slack = counts(Duration::from_millis(20)) as i64;
    assert!(
        (0..slack).contains(&late),
        "one-shot timer test: fired {late} counts after its deadline"
    );
}

/// Write a page that's mapped nowhere else through one kmap, and read it back
/// through another.
fn test_kmap() {
    let pa = port::pagealloc::allocate_physpage().expect("couldn't allocate kmap test page");
    let pattern = |i: usize| (i * 7 + 3) as u8;

    {
        let k = kmap::kmap(pa);
        // SAFETY: kmaps the whole of the newly allocated page, which nothing
        // else refers to.
        let page = unsafe { core::slice::from_raw_parts_mut(k.va() as *mut u8, PAGE_SIZE_4K) };
        page.iter_mut().enumerate().for_each(|(i, b)| *b = pattern(i));
    }

    let k = kmap::kmap(pa);
    // SAFETY: as above.
    let page = unsafe { core::slice::from_raw_parts(k.va() as *const u8, PAGE_SIZE_4K) };
    assert!(page.iter().enumerate().all(|(i, &b)| b == pattern(i)), "kmap test: page differs");
    println!("kmap test: ok");
}

/// Map some pages into a new address space, unmap one, then release it,
/// checking that every page comes back.  A map_zeroed that fails partway
/// leaves nothing behind.
fn test_release() {
    use port::dat::Access::Read;
    let (used_before, _) = port::pagealloc::usage_bytes();
    let space = vm::AddressSpace::new().expect("couldn't allocate address space");
    // SAFETY: nothing refers to memory through the empty address space.
    unsafe { vm::switch(&space) };
    let page = |i| UTZERO + i * PAGE_SIZE_4K;

    segment::map_zeroed::<Arm64>(VirtRange::new(page(4), page(5))).expect("couldn't map a page");
    let (used_mid, _) = port::pagealloc::usage_bytes();
    let overlapping = segment::map_zeroed::<Arm64>(VirtRange::new(page(0), page(5)));
    assert_eq!(overlapping, Err(PutError::AlreadyMapped));
    assert_eq!(port::pagealloc::usage_bytes().0, used_mid, "a failed map_zeroed leaked");
    assert!(!vm::user_page_ok(page(0), Read), "a failed map_zeroed left a page mapped");
    assert!(vm::user_page_ok(page(4), Read));

    segment::map_zeroed::<Arm64>(VirtRange::new(page(0), page(4))).expect("couldn't map pages");
    let pa = Arm64::unput(page(1)).expect("unput found nothing");
    assert!(!vm::user_page_ok(page(1), Read), "unput left the page mapped");
    assert_eq!(Arm64::unput(page(1)), None);
    port::pagealloc::free_physpage(pa).expect("couldn't free the unput page");

    let mut freed = 0;
    // SAFETY: nothing refers to memory through space after this.
    unsafe {
        vm::release(space, |pa| {
            port::pagealloc::free_physpage(pa).expect("couldn't free a page");
            freed += 1;
        })
    }
    .expect("couldn't release address space");
    assert_eq!(freed, 4);
    // SAFETY: the empty address space is refused, and nothing is freed.
    let empty = unsafe { vm::release(vm::AddressSpace::empty(), |_| {}) };
    assert!(matches!(empty, Err(vm::PageTableError::EmptyAddressSpace)));
    assert_eq!(port::pagealloc::usage_bytes().0, used_before);
    println!("release test: ok");
}

static SWTCH_TEST_MAIN: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());
static SWTCH_TEST_OTHER: SyncUnsafeCell<Label> = SyncUnsafeCell::new(Label::new());

/// Switch back and forth between main9 and another kernel context with its
/// own stack, checking that state held across the switches survives.
fn test_swtch() {
    let (used_before, _) = port::pagealloc::usage_bytes();
    let stack = kstack::alloc().expect("couldn't allocate swtch test stack");
    let stack_top = stack.top();

    unsafe { *SWTCH_TEST_OTHER.get() = Label::new_context(swtch_test_other, stack_top) };

    let canary = black_box(0x0123_4567_89ab_cdef_u64);
    for i in 0..3 {
        println!("swtch test: main {i}");
        unsafe { swtch::swtch(SWTCH_TEST_MAIN.get(), SWTCH_TEST_OTHER.get()) };
    }
    assert_eq!(black_box(canary), 0x0123_4567_89ab_cdef);

    // Nothing switches back to the other context, so its stack can be freed.
    // A new stack then reuses the slot, and the pages are all returned.
    // SAFETY: the other context is never resumed.
    unsafe { kstack::free(stack) };
    // Its label now points into a freed stack.  Reset it, so a mistaken
    // switch to it jumps to address 0 and faults.
    unsafe { *SWTCH_TEST_OTHER.get() = Label::new() };
    let again = kstack::alloc().expect("couldn't allocate a second stack");
    assert_eq!(again.top(), stack_top);
    // SAFETY: nothing ran on this stack.
    unsafe { kstack::free(again) };
    assert_eq!(port::pagealloc::usage_bytes().0, used_before);
    println!("swtch test: ok");
}

extern "C" fn swtch_test_other() -> ! {
    let mut i = 0;
    loop {
        println!("swtch test: other {i}");
        i += 1;
        unsafe { swtch::swtch(SWTCH_TEST_OTHER.get(), SWTCH_TEST_MAIN.get()) };
    }
}
