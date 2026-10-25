//! Loading a program into the current process, as Plan 9's sysexec does.
//! There's no exec syscall yet: a start function calls exec, then enters the
//! program with the registers it returns.

use crate::dat::{MMU, Perm, ProcArch, PutError};
use crate::elf::{self, Elf, Segment};
use crate::error::{EBADARG, EBADEXEC, ENOVMEM};
use crate::mem::{PAGE_SIZE_4K, VirtRange};
use crate::pagealloc::{allocate_physpage, free_physpage};
use crate::segment::{map_zeroed, set_bss};
use crate::uaccess::copy_to_user;

/// What a program starts with: its pc and stack pointer, and argc and argv,
/// which each architecture puts in its own registers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UserRegs {
    pub pc: usize,
    pub sp: usize,
    pub argc: usize,
    pub argv: usize,
}

/// Load the ELF executable `image` into the current process, map its stack,
/// and push `argv` onto it, as Plan 9's sysexec does.  Return the registers to
/// enter it with.  The arguments are checked first, as in sysexec, so bad
/// ones map nothing.  As with load, a failure after that isn't undone: the
/// caller must exit the process.
pub fn exec<A: ProcArch>(image: &[u8], argv: &[&[u8]]) -> crate::Result<UserRegs> {
    args_size::<A>(argv)?;
    let pc = load::<A>(image)?;
    map_zeroed::<A>(VirtRange::new(A::USTKTOP - A::USTKSIZE, A::USTKTOP)).map_err(
        |err| match err {
            PutError::AlreadyMapped => EBADEXEC,
            PutError::NoMemory => ENOVMEM,
        },
    )?;
    push_args::<A>(pc, A::USTKTOP, argv)
}

/// Check that `argv` can be pushed onto a stack, and return the size of its
/// strings, NULs included.  The arguments may use at most a quarter of the
/// stack, as in Linux, and a string may not contain a NUL.
fn args_size<A: MMU>(argv: &[&[u8]]) -> crate::Result<usize> {
    if argv.iter().any(|a| a.contains(&0)) {
        return Err(EBADARG);
    }
    let Some(strings) = argv.iter().try_fold(0usize, |n, a| n.checked_add(a.len() + 1)) else {
        return Err(ENOVMEM);
    };
    // A 16-byte aligned top leaves at most 8 bytes of padding below argv.
    let size = strings.next_multiple_of(8).saturating_add(8 * (argv.len() + 1) + 8);
    if size > A::USTKSIZE / 4 {
        return Err(ENOVMEM);
    }
    Ok(strings)
}

/// Push `argv` below `top` in the current user address space: its strings,
/// each NUL-terminated, then below them the array of pointers to them, ending
/// in a null pointer.  Return the registers to start at `pc` with: a stack
/// pointer below the array, 16-byte aligned as AAPCS64 needs, and argc and
/// argv.  If args_size refuses the arguments, nothing is written.
pub fn push_args<A: ProcArch>(pc: usize, top: usize, argv: &[&[u8]]) -> crate::Result<UserRegs> {
    assert!(top.is_multiple_of(16), "push_args: top {top:#x} isn't 16-byte aligned");
    let strings = args_size::<A>(argv)?;
    let argvp = ((top - strings) & !7) - 8 * (argv.len() + 1);
    let mut s = top - strings;
    for (i, a) in argv.iter().enumerate() {
        copy_to_user::<A>(s, a);
        copy_to_user::<A>(s + a.len(), &[0]);
        copy_to_user::<A>(argvp + 8 * i, &(s as u64).to_le_bytes());
        s += a.len() + 1;
    }
    copy_to_user::<A>(argvp + 8 * argv.len(), &0u64.to_le_bytes());
    Ok(UserRegs { pc, sp: argvp & !15, argc: argv.len(), argv: argvp })
}

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
    use crate::fake::{Fake, USTKTOP, mapped, user_mem};

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
    fn push_args_layout() {
        let mut mem = user_mem();
        // Not zero, so the NULs and the null pointer must be written.
        mem.0[..PAGE_SIZE_4K].fill(0xff);
        let base = mem.0.as_mut_ptr() as usize;
        let top = base + PAGE_SIZE_4K;
        let regs = push_args::<Fake>(0x10000, top, &[b"a", b"bc", b""]).expect("push_args");
        let UserRegs { pc, sp, argc, argv } = regs;
        assert_eq!((pc, argc), (0x10000, 3));
        assert_eq!(&mem.0[PAGE_SIZE_4K - 6..PAGE_SIZE_4K], b"a\0bc\0\0", "the strings");
        assert_eq!(argv, top - 8 - 32, "8-aligned below the strings");
        let at = argv - base;
        let ptrs: Vec<u64> = mem.0[at..at + 32]
            .chunks(8)
            .map(|p| u64::from_le_bytes(p.try_into().unwrap()))
            .collect();
        let s = (top - 6) as u64;
        assert_eq!(ptrs, [s, s + 2, s + 5, 0], "pointers to each, then null");
        assert_eq!(sp, argv & !15);
        assert!(sp.is_multiple_of(16));
    }

    #[test]
    fn push_args_empty() {
        let mut mem = user_mem();
        mem.0[..PAGE_SIZE_4K].fill(0xff);
        let top = mem.0.as_mut_ptr() as usize + PAGE_SIZE_4K;
        let UserRegs { sp, argc, argv, .. } = push_args::<Fake>(0, top, &[]).expect("push_args");
        assert_eq!(argc, 0);
        assert_eq!(argv, top - 8);
        assert_eq!(&mem.0[PAGE_SIZE_4K - 8..PAGE_SIZE_4K], &[0; 8], "just the null pointer");
        assert_eq!(sp, top - 16);
    }

    #[test]
    fn push_args_refuses_bad_args() {
        let mut mem = user_mem();
        let top = mem.0.as_mut_ptr() as usize + PAGE_SIZE_4K;
        let big = vec![b'x'; <Fake as MMU>::USTKSIZE / 4];
        assert_eq!(push_args::<Fake>(0, top, &[&big]), Err(ENOVMEM), "a quarter of the stack");
        assert_eq!(push_args::<Fake>(0, top, &[b"a\0b"]), Err(EBADARG), "a NUL in a string");
        assert!(mem.0.iter().all(|&b| b == 0), "something was written");
    }

    #[test]
    fn exec_checks_args_before_loading() {
        // A bad argument is refused before load, which would fail for want of
        // memory on the host.
        assert_eq!(exec::<Fake>(HELLO, &[b"a\0b"]), Err(EBADARG));
        assert!(mapped().is_empty());
    }

    #[test]
    fn load_maps_nothing_without_memory() {
        // The host page allocator has nothing free.
        assert_eq!(load::<Fake>(HELLO), Err(ENOVMEM));
        assert!(mapped().is_empty());
    }
}
