//! Loading a program into the current process, as Plan 9's sysexec does.
//! There's no exec syscall yet: a start function loads its program with load,
//! then enters it.

use crate::dat::{MMU, Perm, ProcArch, PutError};
use crate::elf::{self, Elf, Segment};
use crate::error::{EBADEXEC, ENOVMEM};
use crate::mem::{PAGE_SIZE_4K, VirtRange};
use crate::pagealloc::{allocate_physpage, free_physpage};
use crate::segment::set_bss;

/// Load the ELF executable `image` into the current process: map each of its
/// segments, set its bss to what follows its data, and return its entry point.
/// The layout is checked before anything is mapped.  A failure while mapping
/// isn't undone, as there's no going back after Plan 9's point of no return:
/// the caller must exit the process, whose release frees what was mapped.
pub fn load<A: ProcArch>(image: &[u8]) -> crate::Result<usize> {
    let elf = elf::parse(image).map_err(|_| EBADEXEC)?;
    let bss = layout::<A>(&elf)?;
    for seg in elf.segments() {
        load_segment::<A>(&seg)?;
    }
    set_bss::<A>(bss);
    Ok(elf.entry())
}

/// Check that `elf` fits the user address space, between UTZERO and the guard
/// page below the stack, as text and rodata, then at most one data segment at
/// the top, which the bss follows.  Return the bss.
fn layout<A: MMU>(elf: &Elf) -> crate::Result<VirtRange> {
    let top = A::USTKTOP - A::USTKSIZE - PAGE_SIZE_4K;
    let mut data = None;
    let mut end = A::UTZERO;
    for seg in elf.segments() {
        if data.is_some() || seg.vaddr < A::UTZERO || seg.vaddr + seg.memsz > top {
            return Err(EBADEXEC);
        }
        end = seg.vaddr + seg.memsz;
        if seg.perm == Perm::Data {
            data = Some(seg);
        }
    }
    let pgend = |addr: usize| addr.next_multiple_of(PAGE_SIZE_4K);
    Ok(match data {
        Some(d) => VirtRange::new(d.vaddr + d.file.len(), pgend(d.vaddr + d.memsz)),
        None => VirtRange::new(pgend(end), pgend(end)),
    })
}

/// Map each page `seg` touches, holding its file bytes and zero elsewhere.
fn load_segment<A: MMU>(seg: &Segment) -> crate::Result<()> {
    let start = seg.vaddr & !(PAGE_SIZE_4K - 1);
    let end = (seg.vaddr + seg.memsz).next_multiple_of(PAGE_SIZE_4K);
    let file_end = seg.vaddr + seg.file.len();
    for va in (start..end).step_by(PAGE_SIZE_4K) {
        let pa = allocate_physpage().map_err(|_| ENOVMEM)?;
        A::kmap(pa, |p| {
            // SAFETY: p maps the whole of the newly allocated page, which
            // nothing else refers to.
            let page = unsafe { core::slice::from_raw_parts_mut(p, PAGE_SIZE_4K) };
            page.fill(0);
            let (lo, hi) = (va.max(seg.vaddr), (va + PAGE_SIZE_4K).min(file_end));
            if lo < hi {
                page[lo - va..hi - va].copy_from_slice(&seg.file[lo - seg.vaddr..hi - seg.vaddr]);
            }
        });
        A::put(va, pa, seg.perm).map_err(|err| {
            free_physpage(pa).expect("load: couldn't free an unmapped page");
            match err {
                PutError::AlreadyMapped => EBADEXEC,
                PutError::NoMemory => ENOVMEM,
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, USTKTOP, mapped};

    static HELLO: &[u8] = include_bytes!("../lib/test/elf/hello.elf");

    /// HELLO with program header `i`'s field at `off` replaced by `value`.
    fn patched(i: usize, off: usize, value: u64) -> Vec<u8> {
        let mut image = HELLO.to_vec();
        let at = 64 + i * 56 + off;
        image[at..at + 8].copy_from_slice(&value.to_le_bytes());
        image
    }

    fn layout_of(image: &[u8]) -> crate::Result<VirtRange> {
        layout::<Fake>(&elf::parse(image).expect("didn't parse"))
    }

    #[test]
    fn hello_layout() {
        // The bss starts after the data's file bytes, and ends on a page.
        assert_eq!(layout_of(HELLO), Ok(VirtRange::new(0x13008, 0x15000)));
    }

    #[test]
    fn bad_layouts() {
        let mut low = patched(0, 16, 0x1000);
        low[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        assert_eq!(layout_of(&low), Err(EBADEXEC), "text below UTZERO");
        let guard = (USTKTOP - <Fake as MMU>::USTKSIZE - PAGE_SIZE_4K) as u64;
        assert_eq!(layout_of(&patched(2, 16, guard)), Err(EBADEXEC), "data in the guard page");
        // Rodata made writable: a second data segment.
        let mut two = HELLO.to_vec();
        two[64 + 56 + 4] = 6;
        assert_eq!(layout_of(&two), Err(EBADEXEC), "a segment above the data");
    }

    #[test]
    fn load_maps_nothing_without_memory() {
        // The host page allocator has nothing free.
        assert_eq!(load::<Fake>(HELLO), Err(ENOVMEM));
        assert!(mapped().is_empty());
    }
}
