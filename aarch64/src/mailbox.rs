use crate::io::{read_reg, write_reg};
use crate::registers::rpi_mmio;
use core::mem;
use core::mem::MaybeUninit;
use port::fdt::DeviceTree;
use port::mcslock::{Lock, LockNode};
use port::mem::VirtRange;

const MBOX_READ: usize = 0x00;
const MBOX_STATUS: usize = 0x18;
const MBOX_WRITE: usize = 0x20;

const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;

static MAILBOX: Lock<Option<&'static mut Mailbox>> = Lock::new("mailbox", None);

/// Mailbox init.  Mainly initialises a lock to ensure only one mailbox request
/// can be made at a time.  We have no heap at this point, so creating a mailbox
/// that can be initialised based off the devicetree is rather convoluted.
pub fn init(_dt: &DeviceTree) {
    static mut NODE: LockNode = LockNode::new();
    let mut mailbox = MAILBOX.lock(unsafe { &NODE });
    *mailbox = Some({
        static mut MAYBE_MAILBOX: MaybeUninit<Mailbox> = MaybeUninit::uninit();
        unsafe {
            let mmio = rpi_mmio().expect("mmio base detect failed").to_virt();
            let mbox_range = VirtRange::with_len(mmio + 0xb880, 0x40);

            MAYBE_MAILBOX.write(Mailbox { mbox_range });
            //MAYBE_MAILBOX.write(Mailbox::new(dt, KZERO));
            MAYBE_MAILBOX.assume_init_mut()
        }
    });
}

/// https://developer.arm.com/documentation/ddi0306/b/CHDGHAIG
/// https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface
struct Mailbox {
    pub mbox_range: VirtRange,
}

#[allow(dead_code)]
impl Mailbox {
    fn new(dt: &DeviceTree, mmio_virt_offset: u64) -> Mailbox {
        Mailbox {
            mbox_range: VirtRange::from(
                &dt.find_compatible("brcm,bcm2835-mbox")
                    .next()
                    .and_then(|uart| dt.property_translated_reg_iter(uart).next())
                    .and_then(|reg| reg.regblock())
                    .unwrap()
                    .with_offset(mmio_virt_offset),
            ),
        }
    }

    fn request<T, U>(&self, req: &mut Message<T, U>)
    where
        T: Copy,
        U: Copy,
    {
        // Read status register until full flag not set
        while (read_reg(&self.mbox_range, MBOX_STATUS) & MBOX_FULL) != 0 {}

        // Write the request address combined with the channel to the write register
        let channel = ChannelId::ArmToVc as u32;
        let uart_mbox_u32 = req as *const _ as u32;
        let r = (uart_mbox_u32 & !0xF) | channel;
        write_reg(&self.mbox_range, MBOX_WRITE, r);

        // Wait for response
        // FIXME: two infinite loops - can go awry
        loop {
            while (read_reg(&self.mbox_range, MBOX_STATUS) & MBOX_EMPTY) != 0 {}
            let response = read_reg(&self.mbox_range, MBOX_READ);
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
    tags: T,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Tag<T> {
    tag_id0: TagId,
    tag_buffer_size0: u32,
    tag_code0: u32,
    body: T,
    end_tag: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
union Message<T: Copy, U: Copy> {
    request: Request<T>,
    response: Response<U>,
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

// https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface#tags-arm-to-vc
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
enum TagId {
    GetFirmwareRevision = 0x0000_0001,
    GetBoardModel = 0x0001_0001,
    GetBoardRevision = 0x0001_0002,
    GetBoardMacAddress = 0x0001_0003,
    GetBoardSerial = 0x0001_0004,
    GetArmMemory = 0x0001_0005,
    GetVcMemory = 0x0001_0006,
    SetClockRate = 0x0003_8002,
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

#[allow(dead_code)]
pub fn set_clock_rate(clock_id: u32, rate_hz: u32, skip_setting_turbo: u32) {
    let tags = Tag::<SetClockRateRequest> {
        tag_id0: TagId::SetClockRate,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: SetClockRateRequest { clock_id, rate_hz, skip_setting_turbo },
        end_tag: 0,
    };
    let _: SetClockRateResponse = request(0, &tags);
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct EmptyRequest {}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct MemoryResponse {
    base_addr: u32,
    size: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryInfo {
    pub start: u32,
    pub size: u32,
    pub end: u32,
}

pub fn get_arm_memory() -> MemoryInfo {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetArmMemory,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    let res: MemoryResponse = request(0, &tags);
    let start = res.base_addr;
    let size = res.size;
    let end = start + size;

    MemoryInfo { start, size, end }
}

pub fn get_vc_memory() -> MemoryInfo {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetVcMemory,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    let res: MemoryResponse = request(0, &tags);
    let start = res.base_addr;
    let size = res.size;
    let end = start + size;

    MemoryInfo { start, size, end }
}

pub fn get_firmware_revision() -> u32 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetFirmwareRevision,
        tag_buffer_size0: 4,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, u32>(0, &tags)
}

pub fn get_board_model() -> u32 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardModel,
        tag_buffer_size0: 4,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, u32>(0, &tags)
}

pub fn get_board_revision() -> u32 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardRevision,
        tag_buffer_size0: 4,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, u32>(0, &tags)
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MacAddress {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub f: u8,
}

pub fn get_board_macaddr() -> MacAddress {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardMacAddress,
        tag_buffer_size0: 6,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, MacAddress>(0, &tags)
}

pub fn get_board_serial() -> u64 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardSerial,
        tag_buffer_size0: 8,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    // FIXME: Treating this a `u64` gets us a memory address. Pointer fun ahead.
    // Wrapping in a struct holding a single u64 doesn't work either.
    let res: [u32; 2] = request(0, &tags);
    ((res[0] as u64) << 32) | res[1] as u64
}
