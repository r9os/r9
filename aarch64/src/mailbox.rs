use crate::io::{read_reg, write_reg};
use core::mem;
use core::mem::MaybeUninit;
use port::fdt::{DeviceTree, RegBlock};
use port::mcslock::{Lock, LockNode};

const MBOX_READ: u64 = 0x00;
const MBOX_STATUS: u64 = 0x18;
const MBOX_WRITE: u64 = 0x20;

const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;

static MAILBOX: Lock<Option<&'static mut Mailbox>> = Lock::new("mailbox", None);

/// Mailbox init.  Mainly initialises a lock to ensure only one mailbox request
/// can be made at a time.  We have no heap at this point, so creating a mailbox
/// that can be initialised based off the devicetree is rather convoluted.
pub fn init(dt: &DeviceTree) {
    static mut NODE: LockNode = LockNode::new();
    let mut mailbox = MAILBOX.lock(unsafe { &NODE });
    *mailbox = Some({
        static mut MAYBE_MAILBOX: MaybeUninit<Mailbox> = MaybeUninit::uninit();
        unsafe {
            MAYBE_MAILBOX.write(Mailbox::new(dt));
            MAYBE_MAILBOX.assume_init_mut()
        }
    });
}

/// https://developer.arm.com/documentation/ddi0306/b/CHDGHAIG
/// https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface
struct Mailbox {
    reg: RegBlock,
}

impl Mailbox {
    fn new(dt: &DeviceTree) -> Mailbox {
        Mailbox {
            reg: dt
                .find_compatible("brcm,bcm2835-mbox")
                .next()
                .and_then(|uart| dt.property_translated_reg_iter(uart).next())
                .and_then(|reg| reg.regblock())
                .unwrap(),
        }
    }

    fn request<T, U>(&self, req: &mut Message<T, U>)
    where
        T: Copy,
        U: Copy,
    {
        // Read status register until full flag not set
        while (read_reg(self.reg, MBOX_STATUS) & MBOX_FULL) != 0 {}

        // Write the request address combined with the channel to the write register
        let channel = ChannelId::ArmToVc as u32;
        let uart_mbox_u32 = req as *const _ as u32;
        let r = (uart_mbox_u32 & !0xF) | channel;
        write_reg(self.reg, MBOX_WRITE, r);

        // Wait for response
        // FIXME: two infinite loops - can go awry
        loop {
            while (read_reg(self.reg, MBOX_STATUS) & MBOX_EMPTY) != 0 {}
            let response = read_reg(self.reg, MBOX_READ);
            if response == r {
                break;
            }
        }
    }
}

#[repr(u8)]
enum ChannelId {
    ArmToVc = 8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Request<T> {
    size: u32, // size in bytes
    code: u32, // request code (0)
    tags: T,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Response<T> {
    size: u32, // size in bytes
    code: u32, // response code
    pub tags: T,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Tag<T> {
    tag_id0: u32,
    tag_buffer_size0: u32,
    tag_code0: u32,
    pub body: T,
    end_tag: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
union Message<T: Copy, U: Copy> {
    request: Request<T>,
    pub response: Response<U>,
}

type MessageWithTags<T, U> = Message<Tag<T>, Tag<U>>;

fn request<T, U>(code: u32, tags: &Tag<T>) -> U
where
    T: Copy,
    U: Copy,
{
    let size = mem::size_of::<Message<T, U>>() as u32;
    let req = Request::<Tag<T>> { size, code, tags: *tags };
    let mut msg = MessageWithTags { request: req };
    static mut NODE: LockNode = LockNode::new();
    let mut mailbox = MAILBOX.lock(unsafe { &NODE });
    mailbox.as_deref_mut().unwrap().request(&mut msg);
    let res = unsafe { msg.response };
    res.tags.body
}

#[repr(u32)]
enum TagId {
    GetArmMemory = 0x10005,
    SetClockRate = 0x38002,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateRequest {
    clock_id: u32,
    rate_hz: u32,
    skip_setting_turbo: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateResponse {
    clock_id: u32,
    rate_hz: u32,
}

pub fn set_clock_rate(clock_id: u32, rate_hz: u32, skip_setting_turbo: u32) {
    let tags = Tag::<SetClockRateRequest> {
        tag_id0: TagId::SetClockRate as u32,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: SetClockRateRequest { clock_id, rate_hz, skip_setting_turbo },
        end_tag: 0,
    };
    let _: SetClockRateResponse = request(0, &tags);
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct GetArmMemoryRequest {}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct GetArmMemoryResponse {
    pub base_addr: u32,
    pub size: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ArmMemory {
    pub start: u32,
    pub size: u32,
    pub end: u32,
}

pub fn get_arm_memory() -> ArmMemory {
    let tags = Tag::<GetArmMemoryRequest> {
        tag_id0: TagId::GetArmMemory as u32,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: GetArmMemoryRequest {},
        end_tag: 0,
    };
    let res: GetArmMemoryResponse = request(0, &tags);
    let start = res.base_addr;
    let size = res.size;
    let end = start + size;

    ArmMemory { start, size, end }
}
