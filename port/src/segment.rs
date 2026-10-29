//! User memory, as Plan 9's port/segment.c: pages mapped in the current
//! address space, which a process records as its segments.

use alloc::vec::Vec;

use crate::dat::{Access, MMU, Perm, ProcArch, PutError, UserSegment};
use crate::error::{EBADARG, ENOVMEM, ESOVERLAP};
use crate::mem::{PAGE_SIZE_4K, PhysAddr, VirtRange};
use crate::pagealloc::{allocate_physpage, free_physpage, usage_bytes};
use crate::uaccess::copy_from_user;

/// Copy the current process's `segments` and `bss` into `child`, the address
/// space of a process it's forking, as Plan 9's dupseg does, but every page
/// now, rather than data on write and text shared, which a program can't
/// tell apart.  Each page is read through the current address space,
/// `parent`, into a new page through kmap; then they're all mapped in
/// `child` with it switched in, and `parent` switched back.  A page that
/// isn't mapped is a segment recorded wrongly, which panics.  If pages run
/// out, the copies not yet mapped are freed, and the caller frees `child`,
/// with those that were.
pub(crate) fn copy_segments<A: ProcArch>(
    segments: &[UserSegment],
    bss: Option<VirtRange>,
    parent: &A::Pmmu,
    child: &A::Pmmu,
) -> crate::Result<()> {
    let bss = bss.map(|b| UserSegment {
        range: VirtRange::new(b.start.next_multiple_of(PAGE_SIZE_4K), b.end),
        perm: Perm::Data,
    });
    let mut pages: Vec<(usize, PhysAddr, Perm)> = Vec::new();
    let free = |pages: &[(usize, PhysAddr, Perm)]| {
        for &(_, pa, _) in pages {
            free_physpage(pa).expect("rfork: couldn't free a copied page");
        }
    };
    for s in segments.iter().chain(&bss) {
        for va in (s.range.start..s.range.end).step_by(PAGE_SIZE_4K) {
            assert!(A::user_page_ok(va, Access::Read), "rfork: {s:?} isn't mapped at {va:#x}");
            let Ok(pa) = allocate_physpage() else {
                free(&pages);
                return Err(ENOVMEM);
            };
            A::kmap(pa, |p| {
                // SAFETY: p maps the whole of the newly allocated page, which
                // nothing else refers to.
                let page = unsafe { core::slice::from_raw_parts_mut(p, PAGE_SIZE_4K) };
                copy_from_user::<A>(page, va);
            });
            pages.push((va, pa, s.perm));
        }
    }
    // SAFETY: nothing refers to the parent's user memory while the child's
    // address space is switched in.
    unsafe { A::switch(child) };
    let mut result = Ok(());
    for (i, &(va, pa, perm)) in pages.iter().enumerate() {
        if A::put(va, pa, perm).is_err() {
            free(&pages[i..]);
            result = Err(ENOVMEM);
            break;
        }
    }
    // SAFETY: as above.
    unsafe { A::switch(parent) };
    result
}

/// Set the current process's segments, but for its bss, whose pages the
/// caller has mapped, each as its segment says.
pub fn set_segments<A: ProcArch>(segments: Vec<UserSegment>) {
    let up = A::up();
    assert!(!up.is_null(), "set_segments: no current process");
    // SAFETY: up is the running process, which is setting itself up.
    unsafe { (*up).segments = segments };
}

/// Set the current process's bss to `bss`, whose pages, from its start rounded
/// up to its end, the caller has mapped.
pub fn set_bss<A: ProcArch>(bss: VirtRange) {
    assert!(bss.start <= bss.end, "set_bss: {bss} is reversed");
    assert!(bss.end.is_multiple_of(PAGE_SIZE_4K), "set_bss: {bss} doesn't end on a page");
    let up = A::up();
    assert!(!up.is_null(), "set_bss: no current process");
    // SAFETY: up is the running process, which is setting itself up.
    unsafe { (*up).bss = Some(bss) };
}

/// Move the end of `bss` to `addr`, rounded up to a page, as Plan 9's ibrk.
/// Growing maps zeroed pages, and shrinking unmaps and frees them.  An `addr`
/// of 0 returns the bss's start.  On failure, nothing changes.
pub(crate) fn ibrk<A: MMU>(bss: &mut Option<VirtRange>, addr: usize) -> crate::Result<u64> {
    let Some(bss) = bss else {
        return Err(EBADARG);
    };
    if addr == 0 {
        return Ok(bss.start as u64);
    }
    if addr < bss.start {
        return Err(ENOVMEM);
    }
    let Some(newtop) = addr.checked_next_multiple_of(PAGE_SIZE_4K) else {
        return Err(ENOVMEM);
    };
    // There's one fixed stack, so this is the only other region to collide
    // with.  An unmapped guard page is kept below it, so the stack can't run
    // into the bss.  It also keeps newtop below UADDRTOP.
    if newtop > A::USTKTOP - A::USTKSIZE - PAGE_SIZE_4K {
        return Err(ESOVERLAP);
    }
    if newtop < bss.end {
        unmap_and_free::<A>(VirtRange::new(newtop, bss.end));
        bss.end = newtop;
        return Ok(0);
    }
    // Fail at once, rather than allocating every free page first.
    let (used, total) = usage_bytes();
    if newtop - bss.end > total - used {
        return Err(ENOVMEM);
    }
    match map_zeroed::<A>(VirtRange::new(bss.end, newtop)) {
        Ok(()) => {
            bss.end = newtop;
            Ok(0)
        }
        Err(PutError::AlreadyMapped) => Err(ESOVERLAP),
        Err(PutError::NoMemory) => Err(ENOVMEM),
    }
}

/// Allocate, zero and map a data page for each 4KiB page in `range`, in the
/// current user address space.  Each page is zeroed before it's mapped, so a
/// process never sees another's data.  If it fails partway, the pages it
/// mapped are unmapped and freed again.
pub fn map_zeroed<A: MMU>(range: VirtRange) -> Result<(), PutError> {
    assert!(
        range.start.is_multiple_of(PAGE_SIZE_4K) && range.end.is_multiple_of(PAGE_SIZE_4K),
        "map_zeroed: range {range} isn't page aligned"
    );
    for va in (range.start..range.end).step_by(PAGE_SIZE_4K) {
        let mapped = allocate_physpage().map_err(|_| PutError::NoMemory).and_then(|pa| {
            // SAFETY: f gets the whole of the newly allocated page, which
            // nothing else refers to.
            A::kmap(pa, |p| unsafe { core::ptr::write_bytes(p, 0, PAGE_SIZE_4K) });
            A::put(va, pa, Perm::Data).inspect_err(|_| {
                free_physpage(pa).expect("map_zeroed: couldn't free an unmapped page")
            })
        });
        if let Err(err) = mapped {
            unmap_and_free::<A>(VirtRange::new(range.start, va));
            return Err(err);
        }
    }
    Ok(())
}

/// Unmap and free each 4KiB page mapped in `range`, in the current user
/// address space.  Every page mapped there must belong to the process.
pub fn unmap_and_free<A: MMU>(range: VirtRange) {
    for va in (range.start..range.end).step_by(PAGE_SIZE_4K) {
        if let Some(pa) = A::unput(va) {
            free_physpage(pa).expect("unmap_and_free: couldn't free a page");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, USTKTOP, mapped};

    const BASE: usize = 0x20000;

    #[test]
    fn ibrk_errors() {
        let mut none = None;
        assert_eq!(ibrk::<Fake>(&mut none, BASE), Err(EBADARG), "no bss");

        let bss = VirtRange::new(BASE, BASE + PAGE_SIZE_4K);
        let mut b = Some(bss);
        assert_eq!(ibrk::<Fake>(&mut b, 0), Ok(BASE as u64));
        assert_eq!(ibrk::<Fake>(&mut b, BASE - 1), Err(ENOVMEM), "below the bss");
        let stack = USTKTOP - <Fake as MMU>::USTKSIZE;
        assert_eq!(ibrk::<Fake>(&mut b, stack + 1), Err(ESOVERLAP), "into the stack");
        let guard = stack - PAGE_SIZE_4K;
        assert_eq!(ibrk::<Fake>(&mut b, guard + 1), Err(ESOVERLAP), "into the guard page");
        assert_eq!(ibrk::<Fake>(&mut b, usize::MAX), Err(ENOVMEM), "overflow");
        // The host page allocator has nothing free.
        assert_eq!(ibrk::<Fake>(&mut b, BASE + 2 * PAGE_SIZE_4K), Err(ENOVMEM), "no memory");
        assert_eq!(b, Some(bss));
        assert!(mapped().is_empty());
    }

    #[test]
    fn ibrk_within_the_last_page_changes_nothing() {
        let bss = VirtRange::new(BASE, BASE + PAGE_SIZE_4K);
        let mut b = Some(bss);
        assert_eq!(ibrk::<Fake>(&mut b, BASE + 10), Ok(0));
        assert_eq!(ibrk::<Fake>(&mut b, BASE + PAGE_SIZE_4K), Ok(0));
        assert_eq!(b, Some(bss));
        assert!(mapped().is_empty());
    }

    #[test]
    fn map_zeroed_without_memory_leaves_nothing_mapped() {
        // The host page allocator has nothing free.
        let range = VirtRange::new(0x10000, 0x13000);
        assert_eq!(map_zeroed::<Fake>(range), Err(PutError::NoMemory));
        assert!(mapped().is_empty());
    }

    #[test]
    fn unmap_and_free_skips_unmapped_pages() {
        unmap_and_free::<Fake>(VirtRange::new(0x10000, 0x13000));
        assert!(mapped().is_empty());
    }
}
