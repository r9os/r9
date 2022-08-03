#![allow(unused)]

use core::arch::asm;

// Chapter 5: Legacy Extensions

const SBI_SET_TIMER: usize = 0;
const SBI_CONSOLE_PUTCHAR: usize = 1;
const SBI_CONSOLE_GETCHAR: usize = 2;
const SBI_CLEAR_IPI: usize = 3;
const SBI_SEND_IPI: usize = 4;
const SBI_REMOTE_FENCE_I: usize = 5;
const SBI_REMOTE_SFENCE_VMA: usize = 6;
const SBI_REMOTE_SFENCE_VMA_ASID: usize = 7;
const SBI_SHUTDOWN: usize = 8;

#[inline(always)]
fn sbi_call_legacy(eid: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
    let ret;
    unsafe {
        asm!(
            "ecall",
            inlateout("x10") arg0 => ret,
            in("x11") arg1,
            in("x12") arg2,
            in("x17") eid
        );
    }
    ret
}

#[inline]
pub fn set_timer(timer: usize) {
    sbi_call_legacy(SBI_SET_TIMER, timer, 0, 0);
}

#[inline]
#[deprecated = "expected to be deprecated; no replacement"]
pub fn console_putchar(c: u8) {
    sbi_call_legacy(SBI_CONSOLE_PUTCHAR, c as usize, 0, 0);
}

#[inline]
#[deprecated = "expected to be deprecated; no replacement"]
pub fn console_getchar() -> u8 {
    sbi_call_legacy(SBI_CONSOLE_GETCHAR, 0, 0, 0).try_into().unwrap()
}

#[inline]
pub fn shutdown() -> ! {
    sbi_call_legacy(SBI_SHUTDOWN, 0, 0, 0);
    panic!("shutdown failed!");
}
