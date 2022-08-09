//! SBI interface.
//!
//! Chapter 5: Legacy Extensions

#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code))]

const SBI_SET_TIMER: usize = 0;
const SBI_CONSOLE_PUTCHAR: usize = 1;
const SBI_CONSOLE_GETCHAR: usize = 2;
const _SBI_CLEAR_IPI: usize = 3;
const _SBI_SEND_IPI: usize = 4;
const _SBI_REMOTE_FENCE_I: usize = 5;
const _SBI_REMOTE_SFENCE_VMA: usize = 6;
const _SBI_REMOTE_SFENCE_VMA_ASID: usize = 7;
const SBI_SHUTDOWN: usize = 8;

#[cfg(target_arch = "riscv64")]
fn sbi_call_legacy(eid: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
    let ret;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("x10") arg0 => ret,
            in("x11") arg1,
            in("x12") arg2,
            in("x17") eid
        );
    }
    ret
}

#[cfg(not(target_arch = "riscv64"))]
fn sbi_call_legacy(_eid: usize, _arg0: usize, _arg1: usize, _arg2: usize) -> usize {
    0
}

pub fn _set_timer(timer: usize) {
    sbi_call_legacy(SBI_SET_TIMER, timer, 0, 0);
}

#[deprecated = "expected to be deprecated; no replacement"]
pub fn _consputb(c: u8) {
    sbi_call_legacy(SBI_CONSOLE_PUTCHAR, c as usize, 0, 0);
}

#[deprecated = "expected to be deprecated; no replacement"]
pub fn _consgetb() -> u8 {
    sbi_call_legacy(SBI_CONSOLE_GETCHAR, 0, 0, 0).try_into().unwrap()
}

pub fn shutdown() -> ! {
    sbi_call_legacy(SBI_SHUTDOWN, 0, 0, 0);
    panic!("shutdown failed!");
}
