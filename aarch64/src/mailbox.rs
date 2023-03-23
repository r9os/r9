use crate::io::{read_reg, write_reg};
use core::mem;
use core::mem::MaybeUninit;
use core::ptr;
use port::fdt::{DeviceTree, RegBlock};
use port::mcslock::{Lock, LockNode};

pub const MBOX_READ: u64 = 0x00;
pub const MBOX_STATUS: u64 = 0x18;
pub const MBOX_WRITE: u64 = 0x20;

pub const MBOX_FULL: u32 = 0x8000_0000;
pub const MBOX_EMPTY: u32 = 0x4000_0000;

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

pub struct Mailbox {
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
        let channel = ChannelId::ArmToVc;
        let uart_mbox_u32 = ptr::addr_of!(req) as u32;
        let r = (uart_mbox_u32 & !0xF) | (channel as u32);
        write_reg(self.reg, MBOX_WRITE, r);

        // Wait for response
        loop {
            while (read_reg(self.reg, MBOX_STATUS) & MBOX_EMPTY) != 0 {}
            let response = read_reg(self.reg, MBOX_READ);
            if response == r {
                break;
            }
        }
    }
}

pub fn request<T, U>(req: &mut Message<T, U>)
where
    T: Copy,
    U: Copy,
{
    static mut NODE: LockNode = LockNode::new();
    let mut mailbox = MAILBOX.lock(unsafe { &NODE });
    mailbox.as_deref_mut().unwrap().request(req);
}

#[repr(u32)]
pub enum TagId {
    GetArmMemory = 0x10005,
    SetClockRate = 0x38002,
}

#[repr(u8)]
pub enum ChannelId {
    ArmToVc = 8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Request<T> {
    size: u32, // size in bytes
    code: u32, // request code (0)
    tags: T,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Response<T> {
    size: u32, // size in bytes
    code: u32, // response code
    pub tags: T,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Tag<T> {
    tag_id0: u32,
    tag_buffer_size0: u32,
    tag_code0: u32,
    pub body: T,
    end_tag: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub union Message<T: Copy, U: Copy> {
    request: Request<T>,
    pub response: Response<U>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SetClockRateRequestTag {
    clock_id: u32,
    rate_hz: u32,
    skip_setting_turbo: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SetClockRateResponseTag {
    clock_id: u32,
    rate_hz: u32,
}

pub fn new_set_clock_rate_msg(
    clock_id: u32,
    rate_hz: u32,
    skip_setting_turbo: u32,
) -> Message<Tag<SetClockRateRequestTag>, Tag<SetClockRateResponseTag>> {
    Message::<Tag<SetClockRateRequestTag>, Tag<SetClockRateResponseTag>> {
        request: Request::<Tag<SetClockRateRequestTag>> {
            size: mem::size_of::<Message<SetClockRateRequestTag, SetClockRateResponseTag>>() as u32,
            code: 0,
            tags: Tag::<SetClockRateRequestTag> {
                tag_id0: TagId::SetClockRate as u32,
                tag_buffer_size0: 12,
                tag_code0: 0,
                body: SetClockRateRequestTag { clock_id, rate_hz, skip_setting_turbo },
                end_tag: 0,
            },
        },
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GetArmMemoryRequestTag {}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GetArmMemoryResponseTag {
    pub base_addr: u32,
    pub size: u32,
}

pub fn new_get_arm_memory_msg() -> Message<Tag<GetArmMemoryRequestTag>, Tag<GetArmMemoryResponseTag>>
{
    Message::<Tag<GetArmMemoryRequestTag>, Tag<GetArmMemoryResponseTag>> {
        request: Request::<Tag<GetArmMemoryRequestTag>> {
            size: mem::size_of::<Message<GetArmMemoryRequestTag, GetArmMemoryResponseTag>>() as u32,
            code: 0,
            tags: Tag::<GetArmMemoryRequestTag> {
                tag_id0: TagId::GetArmMemory as u32,
                tag_buffer_size0: 12,
                tag_code0: 0,
                body: GetArmMemoryRequestTag {},
                end_tag: 0,
            },
        },
    }
}
