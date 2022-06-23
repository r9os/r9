extern crate alloc;

use alloc::sync::Arc;
use bitflags::bitflags;
use core::ptr::NonNull;
use core::result::Result;

pub struct Chan {
    offset: u64,
    devoffset: u64,
    typ: u16,
    dev: u32,
    mode: u16,
    flag: u16,
    qid: Qid,
    fid: u32,
    iounit: u32,
    // umh: Option<&Mutex<Mhead>>,
    // umqlock: Obviated by Mutex in umh?
    umc: Option<NonNull<Chan>>,
    uri: usize,
    dri: usize,
    // dirrock: Option<&Mutex<*const ()>>,
    // rockqlock: Obviated by Mutex in dirrock?
    nrock: usize,
    mrock: usize,
    ismtpt: bool,
    // mcp: *mut Mntcache,
    // mux: *mut Mnt,
    aux: *mut (),
    pgrpid: Qid,
    mid: u32,
    // mchan: Arc<Chan>,
    mqid: Qid,
    // path: *const Path,
}

pub struct Device {
    dc: u32,
    name: &'static str,
    attached: bool,
}

bitflags! {
    pub struct Mode: u16 {
        const READ = 0;
        const WRITE = 1;
        const OEXEC = 2;
        const OTRUNC = 4;
        const OCEXEC = 5;
        const ORCLOSE = 6;
        const OEXCL = 12;
    }
}

pub trait Dev {
    fn reset();
    fn init();
    fn shutdown();
    fn attach(spec: &[u8]) -> Chan;
    fn walk(&self, c: &Chan, nc: &mut Chan, name: &[&[u8]]) -> Walkqid;
    fn stat(&self, c: &Chan, sb: &mut [u8]) -> Result<(), ()>;
    fn open(&mut self, c: &Chan, mode: u32) -> Chan;
    fn create(&mut self, c: &mut Chan, name: &[u8], mode: Mode, perms: u32);
    fn close(&mut self, c: Chan);
    fn read(&mut self, c: &mut Chan, buf: &mut [u8], offset: u64) -> Result<u64, ()>;
    fn bread(&mut self, c: &mut Chan, bnum: u64, offset: u64) -> Result<Block, ()>;
    fn write(&mut self, c: &mut Chan, buf: &[u8], offset: u64) -> Result<u64, ()>;
    fn bwrite(&mut self, c: &mut Chan, block: &Block, offset: u64) -> Result<u64, ()>;
    fn remove(&mut self, c: &mut Chan);
    fn wstat(&mut self, c: &mut Chan, sb: &[u8]) -> Result<(), ()>;
    fn power(&mut self, on: bool);
    fn config(&mut self /* other args */) -> Result<(), ()>;
}

pub struct Block {
    // TODO(cross): block linkage.
    rp: usize,
    wp: usize,
    lim: *const u8,
    base: *mut u8,
    flag: u16,
    checksum: u16,
    magic: u32,
}

pub struct Qid {
    path: u64,
    vers: u32,
    typ: u8,
}

pub struct Walkqid {
    clone: Arc<Chan>,
    qids: [Qid],
}
