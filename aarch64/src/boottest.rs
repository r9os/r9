//! The kernel's boot-time tests: kmap, swtch, releasing an address space,
//! and a set of test processes, most of them small assembled programs, which
//! check the syscalls, user memory access and the ELF loader.

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
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use port::dat::{MMU, Perm, PutError};
use port::exec::UserRegs;
use port::mem::{PAGE_SIZE_4K, PhysAddr, VirtRange};
use port::println;
use port::segment;

/// Run the tests, panicking if one fails.
pub fn run() {
    test_clock();
    test_timer_interrupt();
    test_unhandled_interrupt();
    test_kmap();
    test_swtch();
    test_release();
    test_user_processes();
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
    // wfi, which SCTLR_EL1.nTWI traps at EL0.
    ".balign 8",
    "usertest_wfi:",
    "   wfi",
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
        UserTest { name: "argv", start: argv_start, status: b"" },
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

/// The argv test program's start: its arguments are pushed onto its stack.
fn argv_start() {
    load_user(usertext!(usertest_argv, usertest_argv_end));
    let regs = match port::exec::push_args::<Arm64>(UTZERO, USTKTOP, &[b"argv", b"ok\n"]) {
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
    let now = || port::clock::now().expect("clock test: no clock");
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
    let hz = port::clock::frequency().expect("no clock");
    let now = || port::clock::now().expect("no clock");
    let end = now() + port::clock::to_counts(d, hz);
    // SAFETY: unmasking IRQs lets the GIC's enabled interrupts in, and only
    // the test's is enabled.  Neither asm! says it leaves memory alone, so
    // each is a compiler barrier.
    unsafe { core::arch::asm!("msr daifclr, #2", options(nostack)) };
    while now() < end {
        core::hint::spin_loop();
    }
    unsafe { core::arch::asm!("msr daifset, #2", options(nostack)) };
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
