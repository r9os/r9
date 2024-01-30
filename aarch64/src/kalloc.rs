use crate::vm::{Page4K, PAGE_SIZE_4K};
use core::ptr;
use port::mcslock::{Lock, LockNode};

static FREE_LIST: Lock<FreeList> = Lock::new("kmem", FreeList { next: None });

#[repr(align(4096))]
struct FreeList {
    next: Option<ptr::NonNull<FreeList>>,
}
unsafe impl Send for FreeList {}

#[derive(Debug)]
pub enum Error {
    NoFreeBlocks,
}

impl FreeList {
    pub fn put(&mut self, page: &mut Page4K) {
        let ptr = (page as *mut Page4K).addr();
        assert_eq!(ptr % PAGE_SIZE_4K, 0, "freeing unaligned page");
        page.scribble();
        let f = page as *mut Page4K as *mut FreeList;
        unsafe {
            ptr::write(f, FreeList { next: self.next });
        }
        self.next = ptr::NonNull::new(f);
    }

    pub fn get(&mut self) -> Result<&'static mut Page4K, Error> {
        let mut next = self.next.ok_or(Error::NoFreeBlocks)?;
        let next = unsafe { next.as_mut() };
        self.next = next.next;
        let pg = unsafe { &mut *(next as *mut FreeList as *mut Page4K) };
        pg.clear();
        Ok(pg)
    }
}

pub unsafe fn free_pages(pages: &mut [Page4K]) {
    static mut NODE: LockNode = LockNode::new();
    let mut lock = FREE_LIST.lock(unsafe { &*ptr::addr_of!(NODE) });
    let fl = &mut *lock;
    for page in pages.iter_mut() {
        fl.put(page);
    }
}

pub fn alloc() -> Result<&'static mut Page4K, Error> {
    static mut NODE: LockNode = LockNode::new();
    let mut lock = FREE_LIST.lock(unsafe { &*ptr::addr_of!(NODE) });
    let fl = &mut *lock;
    fl.get()
}
