pub mod devcons;

use crate::paging::PageTable;
use core::ptr::null_mut;

pub const PHYSICAL_MEMORY_OFFSET: usize = 0xFFFF_FFFF_C000_0000;

pub const PGSIZE: usize = 4096; // bytes per page
pub const PGSHIFT: usize = 12; // bits of offset within a page
pub const PGMASKLEN: usize = 9;
pub const PGMASK: usize = 0x1FF;

pub fn platform_init() {}
