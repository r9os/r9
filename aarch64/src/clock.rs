//! The generic timer.  l.S puts the timer's controls in a known state.

#[cfg(not(test))]
use aarch64_cpu::registers::{
    CNTFRQ_EL0, CNTV_CTL_EL0, CNTV_CVAL_EL0, CNTVCT_EL0, Readable, Writeable,
};
use core::sync::atomic::{AtomicUsize, Ordering};
use port::fdt::DeviceTree;

/// The virtual timer's INTID, from the device tree.
static VTIMER_INTID: AtomicUsize = AtomicUsize::new(0);

/// port::clock's functions.  Reads of CNTVCT_EL0 are in program order
/// relative to each other, but can be taken ahead of earlier instructions
/// (Arm ARM DDI 0487 M.d, D12.2.2), so an isb keeps a time read after some
/// work from being read before it, as Linux's __arch_counter_get_cntvct
/// does.  Firmware sets CNTFRQ_EL0 (D24.10.1).  The virtual timer interrupts
/// while it's enabled and the counter has reached CNTV_CVAL_EL0 (D24.10.28,
/// D24.10.29).  Its interrupt is level-triggered, so set_timer's isb makes
/// sure a new deadline, or disabling it, has quietened it before the GIC's
/// EOI, as Linux's arch_timer_reg_write does.
#[cfg(not(test))]
static CLOCK: port::clock::Ops = port::clock::Ops {
    now: || {
        aarch64_cpu::asm::barrier::isb(aarch64_cpu::asm::barrier::SY);
        CNTVCT_EL0.get()
    },
    frequency: || CNTFRQ_EL0.get(),
    set_timer: |when| {
        match when {
            Some(when) => {
                CNTV_CVAL_EL0.set(when);
                CNTV_CTL_EL0.set(1);
            }
            None => CNTV_CTL_EL0.set(0),
        }
        aarch64_cpu::asm::barrier::isb(aarch64_cpu::asm::barrier::SY);
    },
};

/// Register the counter with port::clock, and find the virtual timer's
/// interrupt.
pub fn init(dt: &DeviceTree) {
    #[cfg(not(test))]
    port::clock::init(&CLOCK);
    VTIMER_INTID.store(vtimer_intid_from(dt), Ordering::Relaxed);
}

/// The virtual timer's INTID.
pub fn vtimer_intid() -> usize {
    VTIMER_INTID.load(Ordering::Relaxed)
}

/// The virtual timer's INTID: the timer node's interrupts are the secure
/// physical, non-secure physical, virtual and hypervisor timers' (the
/// arm,armv8-timer binding), each as a GIC interrupt's three cells: type,
/// number and flags.  The virtual timer's is a PPI (type 1), and a PPI's
/// INTID is 16 plus its number (IHI 0048B.b, 2.2.1).
fn vtimer_intid_from(dt: &DeviceTree) -> usize {
    let node =
        dt.find_compatible("arm,armv8-timer").next().expect("clock: no arm,armv8-timer node");
    let interrupts = dt.property(&node, "interrupts").expect("clock: timer has no interrupts");
    const VIRTUAL: usize = 2;
    const CELLS: usize = 3;
    let mut cells = dt.property_value_as_u32_iter(&interrupts).skip(VIRTUAL * CELLS);
    match (cells.next(), cells.next()) {
        (Some(1), Some(n)) if n < 16 => 16 + n as usize,
        cells => panic!("clock: virtual timer's interrupt isn't a PPI: {cells:?}"),
    }
}

/// Take the virtual timer's interrupt for the timer queue, and start the HZ
/// clock.
pub fn start() {
    crate::gic::attach(vtimer_intid(), port::timer::interrupt);
    port::timer::start();
}
