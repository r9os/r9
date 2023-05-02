pub mod devcons;
use port::println;

pub const PHYSICAL_MEMORY_OFFSET: usize = 0xFFFF_FFFF_4000_0000;

pub fn platform_init() {
    println!("platform_init");
}
