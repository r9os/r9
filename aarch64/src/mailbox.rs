use crate::deviceutil::map_device_register;
use crate::io::{read_reg, write_reg};
use crate::vm;
use port::Result;
use port::fdt::DeviceTree;
use port::mcslock::{Lock, LockNode};
use port::mem::{PhysAddr, PhysRange, VirtRange};

#[cfg(not(test))]
use port::println;

const MBOX_READ: usize = 0x00;
const MBOX_STATUS: usize = 0x18;
const MBOX_WRITE: usize = 0x20;

const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;

static MAILBOX: Lock<Option<Mailbox>> = Lock::new("mailbox", None);

/// Mailbox init.  Mainly initialises a lock to ensure only one mailbox request
/// can be made at a time.
pub fn init(dt: &DeviceTree) {
    match Mailbox::new(dt) {
        Ok(mbox) => {
            let node = LockNode::new();
            let mut mailbox = MAILBOX.lock(&node);
            *mailbox = Some(mbox);
        }
        Err(msg) => {
            println!("can't initialise mailbox: {:?}", msg);
        }
    }
}

/// https://developer.arm.com/documentation/ddi0306/b/CHDGHAIG
/// https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface
struct Mailbox {
    pub mbox_virtrange: VirtRange,
}

impl Mailbox {
    fn new(dt: &DeviceTree) -> Result<Self> {
        let mbox_physrange = Self::find_mbox_physrange(dt)?;
        match map_device_register("mailbox", mbox_physrange, vm::PageSize::Page4K) {
            Ok(mbox_virtrange) => Ok(Mailbox { mbox_virtrange }),
            Err(msg) => {
                println!("can't map mailbox {:?}", msg);
                Err("can't create mailbox")
            }
        }
    }

    /// Get the physical range required for this device
    fn find_mbox_physrange(dt: &DeviceTree) -> Result<PhysRange> {
        dt.find_compatible("brcm,bcm2835-mbox")
            .next()
            .and_then(|uart| dt.property_translated_reg_iter(uart).next())
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .ok_or("can't find mbox")
    }

    fn request<T, U>(&self, req: &mut Message<T, U>)
    where
        T: Copy,
        U: Copy,
    {
        // Read status register until full flag not set
        while (read_reg(&self.mbox_virtrange, MBOX_STATUS) & MBOX_FULL) != 0 {}

        // Write the request address combined with the channel to the write register
        let channel = ChannelId::ArmToVc as u32;
        let uart_mbox_u32 = req as *const _ as u32;
        let r = (uart_mbox_u32 & !0xF) | channel;
        write_reg(&self.mbox_virtrange, MBOX_WRITE, r);

        // Wait for response
        // FIXME: two infinite loops - can go awry
        loop {
            while (read_reg(&self.mbox_virtrange, MBOX_STATUS) & MBOX_EMPTY) != 0 {}
            let response = read_reg(&self.mbox_virtrange, MBOX_READ);
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
    let size = size_of::<Message<T, U>>() as u32;
    let req = Request::<Tag<T>> { size, code, tags: *tags };
    let mut msg = MessageWithTags { request: req };
    let node = LockNode::new();
    MAILBOX
        .lock(&node)
        .as_mut()
        .map(|mb| {
            mb.request(&mut msg);
            unsafe { msg.response.tags.body }
        })
        .expect("mailbox not initialised")
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
#[allow(dead_code)]
pub struct MemoryInfo {
    pub start: u32,
    pub size: u32,
    pub end: u32,
}

#[allow(dead_code)]
pub fn get_arm_memory() -> PhysRange {
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

    PhysRange::new(PhysAddr::new(start as u64), PhysAddr::new(end as u64))
}

#[allow(dead_code)]
pub fn get_vc_memory() -> PhysRange {
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

    PhysRange::new(PhysAddr::new(start as u64), PhysAddr::new(end as u64))
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
