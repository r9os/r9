#![feature(alloc_error_handler)]
#![feature(naked_functions)]
#![feature(sync_unsafe_cell)]
#![cfg_attr(not(any(test)), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod dat;
mod devcons;
mod pio;
mod proc;
mod uart16550;

use proc::{swtch, Label};

#[cfg(not(test))]
core::arch::global_asm!(include_str!("l.S"), options(att_syntax));

use port::println;

static mut THRSTACK: [u64; 1024] = [0; 1024];
static mut CTX: u64 = 0;
static mut THR: u64 = 0;

fn jumpback() {
    println!("in a thread");
    unsafe {
        let thr = &mut *(THR as *mut Label);
        let ctx = &mut *(CTX as *mut Label);
        swtch(thr, ctx);
    }
}

#[no_mangle]
pub extern "C" fn main9() {
    devcons::init();
    println!();
    println!("r9 from the Internet");
    println!("looping now");
    let mut ctx = Label::new();
    let mut thr = Label::new();
    thr.pc = jumpback as usize as u64;
    unsafe {
        thr.sp = &mut THRSTACK[1023] as *mut _ as u64;
        CTX = &mut ctx as *mut _ as u64;
        THR = &mut thr as *mut _ as u64;
        swtch(&mut ctx, &mut thr);
    }
    println!("came out the other side of a context switch");
    #[allow(clippy::empty_loop)]
    loop {}
}

mod runtime;
