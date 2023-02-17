use core::mem;

pub const MBOX_READ: u64 = 0x00;
pub const MBOX_STATUS: u64 = 0x18;
pub const MBOX_WRITE: u64 = 0x20;

pub const MBOX_FULL: u32 = 0x8000_0000;
pub const MBOX_EMPTY: u32 = 0x4000_0000;

#[repr(u32)]
pub enum TagId {
    GetClockRate = 0x38002,
}

#[repr(u8)]
pub enum ChannelId {
    ArmToVc = 8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateRequest {
    size: u32, // size in bytes
    code: u32, // request code (0)

    // Tag
    tag_id0: u32,
    tag_buffer_size0: u32,
    tag_code0: u32,
    clock_id: u32,
    rate_hz: u32,
    skip_setting_turbo: u32,
    // No tag padding
    end_tag: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateResponse {
    size: u32, // size in bytes
    code: u32, // response code

    // Tag
    tag_id0: u32,
    tag_buffer_size0: u32,
    tag_code0: u32,
    clock_id: u32,
    rate_hz: u32,
    // No tag padding
    end_tag: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub union SetClockRate {
    request: SetClockRateRequest,
    response: SetClockRateResponse,
}

impl SetClockRate {
    pub fn new(clock_id: u32, rate_hz: u32, skip_setting_turbo: u32) -> Self {
        SetClockRate {
            request: SetClockRateRequest {
                size: mem::size_of::<SetClockRateRequest>() as u32,
                code: 0,
                tag_id0: TagId::GetClockRate as u32,
                tag_buffer_size0: 12,
                tag_code0: 0,
                clock_id,
                rate_hz,
                skip_setting_turbo,
                end_tag: 0,
            },
        }
    }
}
