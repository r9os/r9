extern crate alloc;

use alloc::sync::Arc;
use bitflags::bitflags;
use core::ptr::NonNull;
use core::result::Result;

pub struct Chan {
    _offset: u64,
    _devoffset: u64,
    _typ: u16,
    _dev: u32,
    _mode: u16,
    _flag: u16,
    _qid: Qid,
    _fid: u32,
    _iounit: u32,
    // umh: Option<&Mutex<Mhead>>,
    // umqlock: Obviated by Mutex in umh?
    _umc: Option<NonNull<Chan>>,
    _uri: usize,
    _dri: usize,
    // dirrock: Option<&Mutex<*const ()>>,
    // rockqlock: Obviated by Mutex in dirrock?
    _nrock: usize,
    _mrock: usize,
    _ismtpt: bool,
    // mcp: *mut Mntcache,
    // mux: *mut Mnt,
    _aux: *mut (),
    _pgrpid: Qid,
    _mid: u32,
    // mchan: Arc<Chan>,
    _mqid: Qid,
    // path: *const Path,
}

pub struct Device {
    _dc: u32,
    _name: &'static str,
    _attached: bool,
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
    _rp: usize,
    _wp: usize,
    _lim: *const u8,
    _base: *mut u8,
    _flag: u16,
    _checksum: u16,
    _magic: u32,
}

pub struct Qid {
    _path: u64,
    _vers: u32,
    _typ: u8,
}

pub struct Walkqid {
    _clone: Arc<Chan>,
    _qids: [Qid],
}
