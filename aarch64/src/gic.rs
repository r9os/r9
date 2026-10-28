//! The GIC-400 interrupt controller, a GICv2 (Arm IHI 0048B.b).  It's set up
//! as Linux's irq-gic.c sets one up, for the one CPU r9 runs on.

use crate::deviceutil::map_device_register;
use crate::io::{read_reg, write_reg};
use crate::vm::PageSize;
use port::fdt::{DeviceTree, TranslatedReg};
use port::iprintln;
use port::mcslock::{Lock, LockNode};
use port::mem::{PhysRange, VirtRange};
use port::println;

// Distributor registers.  Each n-indexed register holds a bit for each of 32
// interrupts, a byte for each of 4, or 2 bits for each of 16.
const GICD_CTLR: usize = 0x000; // 4.3.1
const GICD_TYPER: usize = 0x004; // 4.3.2
const GICD_ISENABLER: usize = 0x100; // 4.3.5
const GICD_ICENABLER: usize = 0x180; // 4.3.6
const GICD_ICPENDR: usize = 0x280; // 4.3.8
const GICD_ISACTIVER: usize = 0x300; // 4.3.9
const GICD_ICACTIVER: usize = 0x380; // 4.3.10
const GICD_IPRIORITYR: usize = 0x400; // 4.3.11
const GICD_ITARGETSR: usize = 0x800; // 4.3.12
const GICD_ICFGR: usize = 0xc00; // 4.3.13

// CPU interface registers.
const GICC_CTLR: usize = 0x00; // 4.4.1
const GICC_PMR: usize = 0x04; // 4.4.2
const GICC_IAR: usize = 0x0c; // 4.4.4
const GICC_EOIR: usize = 0x10; // 4.4.5
const GICC_IIDR: usize = 0xfc; // 4.4.14

/// INTIDs 0-31 are each CPU's SGIs and PPIs, and SPIs start here (2.2.1).
const NPRIVATE: usize = 32;

/// The largest number of INTIDs: 1020 and up are special (2.2.1).
const MAXINTID: usize = 1020;

/// Every interrupt's priority, and the mask that lets them all through, are
/// Linux's (GICD_INT_DEF_PRI and GICC_INT_PRI_THRESHOLD).  A non-secure write
/// under the Security Extensions sees priorities shifted (3.5.1), and these
/// values still leave every interrupt above the mask.
const PRIORITY: u32 = 0xa0;
const PRIORITY_MASK: u32 = 0xf0;

/// The GIC's mapped registers, and the handler for each interrupt.  The lock
/// is taken with IRQs masked, and held only to copy them out.
static GIC: Lock<Gic> = Lock::new("gic", Gic { regs: None, handlers: [None; MAXINTID] });

struct Gic {
    regs: Option<Regs>,
    handlers: [Option<fn()>; MAXINTID],
}

#[derive(Clone, Copy)]
struct Regs {
    dist: VirtRange,
    cpu: VirtRange,
    nintid: usize,
}

/// Find the GIC in the device tree, check it's a GICv2, and set it up with
/// every interrupt disabled.  Writing 1 to each CTLR enables delivery
/// whether the GIC has the Security Extensions, as on a Pi 4, where firmware
/// makes every interrupt non-secure Group 1, or not, as on QEMU, where every
/// interrupt is Group 0 (4.3.1, 4.4.1).
pub fn init(dt: &DeviceTree) {
    let node = dt.find_compatible("arm,gic-400").next().expect("gic: no arm,gic-400 node");
    let mut regs = dt.property_translated_reg_iter(node);
    let dist = map(regs.next(), "gicd");
    let cpu = map(regs.next(), "gicc");

    let iidr = read_reg(&cpu, GICC_IIDR);
    assert!((iidr >> 16) & 0xf == 2, "gic: not a GICv2 (GICC_IIDR {iidr:#x})");

    let lines = (read_reg(&dist, GICD_TYPER) & 0x1f) as usize;
    let nintid = (NPRIVATE * (lines + 1)).min(MAXINTID);

    init_dist(&dist, nintid);
    init_cpu(&dist, &cpu);
    let node = LockNode::new();
    GIC.ilock(&node).regs = Some(Regs { dist, cpu, nintid });
    println!("GIC: {nintid} interrupts");
}

/// Handle the interrupt that's pending, as Plan 9's irq: acknowledge it, call
/// its handler, and end it, one interrupt per exception.  A handler runs with
/// IRQs masked, mustn't allocate, and must quieten a level-triggered source
/// before it returns, or it's taken again at once.  An interrupt with no
/// handler is disabled, so it can't recur, then reported.
pub fn irq() {
    let node = LockNode::new();
    let (regs, iar, handler) = {
        let gic = GIC.ilock(&node);
        let regs = gic.regs.expect("gic: interrupt before init");
        let iar = read_reg(&regs.cpu, GICC_IAR);
        let handler = gic.handlers.get(iar_intid(iar)).copied().flatten();
        (regs, iar, handler)
    };
    let intid = iar_intid(iar);
    // 1020-1023 are special, and need no EOI: 1023 is spurious (1.4.4).
    if intid >= MAXINTID {
        return;
    }
    match handler {
        Some(f) => f(),
        None => {
            set_bit(&regs.dist, GICD_ICENABLER, intid);
            iprintln!("gic: interrupt {intid} has no handler: disabled");
        }
    }
    write_reg(&regs.cpu, GICC_EOIR, iar);
}

/// The INTID in a GICC_IAR value, without the source CPU of an SGI (4.4.4).
fn iar_intid(iar: u32) -> usize {
    (iar & 0x3ff) as usize
}

/// Call `f` for interrupt `intid`, and enable it, as Plan 9's intrenable.
pub fn attach(intid: usize, f: fn()) {
    let node = LockNode::new();
    let mut gic = GIC.ilock(&node);
    let regs = valid(&gic, intid);
    assert!(gic.handlers[intid].is_none(), "gic: interrupt {intid} already has a handler");
    gic.handlers[intid] = Some(f);
    set_bit(&regs.dist, GICD_ISENABLER, intid);
}

/// Disable interrupt `intid`, and forget its handler.
pub fn detach(intid: usize) {
    let node = LockNode::new();
    let mut gic = GIC.ilock(&node);
    let regs = valid(&gic, intid);
    set_bit(&regs.dist, GICD_ICENABLER, intid);
    gic.handlers[intid] = None;
}

/// Enable interrupt `intid` without a handler, so that taking it disables it.
pub fn enable(intid: usize) {
    let node = LockNode::new();
    let regs = valid(&GIC.ilock(&node), intid);
    set_bit(&regs.dist, GICD_ISENABLER, intid);
}

/// Whether interrupt `intid` is enabled.
pub fn enabled(intid: usize) -> bool {
    get_bit(GICD_ISENABLER, intid)
}

/// Whether interrupt `intid` is active: taken, and not yet ended.
pub fn active(intid: usize) -> bool {
    get_bit(GICD_ISACTIVER, intid)
}

/// The GIC's registers, once `intid` is checked to be one of its interrupts.
fn valid(gic: &Gic, intid: usize) -> Regs {
    let regs = gic.regs.expect("gic: used before init");
    assert!(intid < regs.nintid, "gic: no interrupt {intid}");
    regs
}

/// Write interrupt `intid`'s bit in a register that has one bit for each of
/// 32 interrupts, and where writing 0 has no effect, such as GICD_ISENABLERn.
fn set_bit(dist: &VirtRange, reg: usize, intid: usize) {
    write_reg(dist, reg + intid / 32 * 4, 1 << (intid % 32));
}

/// Read interrupt `intid`'s bit in a one-bit-per-interrupt register.
fn get_bit(reg: usize, intid: usize) -> bool {
    let node = LockNode::new();
    let regs = valid(&GIC.ilock(&node), intid);
    read_reg(&regs.dist, reg + intid / 32 * 4) & 1 << (intid % 32) != 0
}

/// Map the first page of one of the GIC's reg ranges, which is all r9 uses:
/// the CPU interface's second page holds only GICC_DIR (4.1.3), for split
/// EOI.
fn map(reg: Option<TranslatedReg>, name: &'static str) -> VirtRange {
    let reg = reg.and_then(|r| r.regblock()).unwrap_or_else(|| panic!("gic: no {name} reg"));
    let range = PhysRange::from(&reg);
    let page = PageSize::Page4K.size();
    assert!(range.size() >= page, "gic: {name} reg too small ({range:?})");
    let range = PhysRange::with_pa_len(range.start, page);
    map_device_register(name, range, PageSize::Page4K)
        .unwrap_or_else(|e| panic!("gic: can't map {name}: {e}"))
}

/// Disable the distributor, make every SPI level-triggered, priority
/// PRIORITY, targeted at this CPU, disabled, not pending and not active, then
/// enable it.
fn init_dist(dist: &VirtRange, nintid: usize) {
    write_reg(dist, GICD_CTLR, 0);
    let target = cpu_mask(dist) * 0x0101_0101;
    for i in (NPRIVATE..nintid).step_by(32) {
        write_reg(dist, GICD_ICENABLER + i / 8, !0);
        write_reg(dist, GICD_ICPENDR + i / 8, !0);
        write_reg(dist, GICD_ICACTIVER + i / 8, !0);
    }
    for i in (NPRIVATE..nintid).step_by(16) {
        write_reg(dist, GICD_ICFGR + i / 4, 0);
    }
    for i in (NPRIVATE..nintid).step_by(4) {
        write_reg(dist, GICD_IPRIORITYR + i, PRIORITY * 0x0101_0101);
        write_reg(dist, GICD_ITARGETSR + i, target);
    }
    write_reg(dist, GICD_CTLR, 1);
}

/// Disable this CPU's SGIs and PPIs, clear them, give them priority PRIORITY,
/// then unmask every priority and enable the CPU interface.  These
/// distributor registers are banked per CPU.
fn init_cpu(dist: &VirtRange, cpu: &VirtRange) {
    write_reg(dist, GICD_ICENABLER, !0);
    write_reg(dist, GICD_ICPENDR, !0);
    write_reg(dist, GICD_ICACTIVER, !0);
    for i in (0..NPRIVATE).step_by(4) {
        write_reg(dist, GICD_IPRIORITYR + i, PRIORITY * 0x0101_0101);
    }
    write_reg(cpu, GICC_PMR, PRIORITY_MASK);
    write_reg(cpu, GICC_CTLR, 1);
}

/// This CPU's bit in GICD_ITARGETSR, found as Linux's gic_get_cpumask finds
/// it: GICD_ITARGETSR0-7 read as this CPU's bit for each interrupt that has
/// one.  A GIC with one CPU interface reads them as zero, and ignores writes
/// to the rest (4.3.12).
fn cpu_mask(dist: &VirtRange) -> u32 {
    (0..NPRIVATE)
        .step_by(4)
        .map(|i| read_reg(dist, GICD_ITARGETSR + i))
        .map(|m| (m | m >> 8 | m >> 16 | m >> 24) & 0xff)
        .find(|&m| m != 0)
        .unwrap_or(0)
}
