//! Loading a program into the current process, as Plan 9's sysexec does:
//! from a file, for the EXEC syscall, or from an image in the kernel, for a
//! process the kernel starts, whose start function enters the program with
//! the registers exec returns.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::chan::{Chan, NOPEN, OEXEC};
use crate::dat::{Access, MMU, Perm, ProcArch, PutError, UserRegs, UserSegment};
use crate::dev::open_name;
use crate::elf::{self, Elf, Segment};
use crate::error::{EBADARG, EBADEXEC, ENOVMEM, Error};
use crate::fixedstring::FixedString;
use crate::mem::{PAGE_SIZE_4K, VirtRange};
use crate::pagealloc::{allocate_physpage, free_physpage};
use crate::proc::pexit;
use crate::segment::{map_zeroed, set_bss, set_segments};
use crate::uaccess::{
    bad_address, copy_from_user, copy_to_user, try_copy_str_from_user, user_range_ok,
};

/// How much more of a file the EXEC syscall reads at a time, once it has
/// read the file's length, for a file that's longer or has none.
const CHUNK: usize = 64 * 1024;

/// Exec the file `name`, with the arguments in the null-terminated array of
/// pointers at user address `argv`, as Plan 9's sysexec, and return the
/// registers to enter it with.  Before the point of no return, the file is
/// opened OEXEC, read whole and its layout checked, and the arguments copied
/// in, and a failure returns, leaving the process as it was.  Past it, the descriptors opened
/// OCEXEC are closed, and the image loaded into a new address space, which
/// replaces the old.  Plan 9 can't fail there, as it pages on demand, but
/// r9 maps eagerly, so a failure there exits the process with the error.
pub(crate) fn sysexec<A: ProcArch>(name: String, argv: usize) -> Result<UserRegs, Error> {
    let c = open_name(&name, OEXEC)?;
    let image = read_image(&c, A::USTKTOP - A::USTKSIZE - PAGE_SIZE_4K - A::UTZERO)?;
    drop(c);
    layout::<A>(&elf::parse(&image).map_err(|_| EBADEXEC)?)?;
    let Some(args) = copy_argv::<A>(argv) else {
        drop((image, name));
        bad_address::<A>()
    };
    let args = args?;
    let pmmu = A::newpmmu().map_err(|_| ENOVMEM)?;
    close_cexec::<A>();
    replace_pmmu::<A>(pmmu);
    // What's left in the kernel is freed before a failure exits.
    let r = {
        let argv: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
        exec::<A>(&image, &name, &argv)
    };
    drop((args, image, name));
    match r {
        Ok(regs) => Ok(regs),
        Err(err) => pexit::<A>(err.as_bytes()),
    }
}

/// The file `c` holds, read whole through its device, at most `max` bytes:
/// a bigger file, by its length or by what's read, is Ebadexec, as
/// sysexec's check of the header's text size against the user address
/// space.  The buffer is the file's length, as its device set it on open,
/// and a byte more, so the read that finds the end doesn't grow it: an image
/// is most of a megabyte, and copying one as it grows could exhaust the
/// heap.  The heap is the bound that matters, as `max` is most of the
/// address space: an image that won't fit is Enovmem.
fn read_image(c: &Arc<Chan>, max: usize) -> crate::Result<Vec<u8>> {
    let length = usize::try_from(c.length()).unwrap_or(usize::MAX);
    if length > max {
        return Err(EBADEXEC);
    }
    let mut image = Vec::new();
    loop {
        let len = image.len();
        if len > max {
            return Err(EBADEXEC);
        }
        let want = if len <= length { (length - len).saturating_add(1) } else { CHUNK };
        let want = want.min(max + 1 - len);
        image.try_reserve_exact(want).map_err(|_| ENOVMEM)?;
        image.resize(len + want, 0);
        let n = c.dev().read(c, &mut image[len..], len as u64)? as usize;
        image.truncate(len + n);
        if n == 0 {
            return Ok(image);
        }
    }
}

/// The strings the null-terminated array of pointers at user address `argv`
/// points to, copied in, as sysexec's passes over it, but stopping with
/// Enovmem as soon as they won't fit on the new stack, as args_size checks.
/// None for a bad address, which kills the process, as sysexec's validaddr,
/// once the caller has freed what it holds.
fn copy_argv<A: ProcArch>(argv: usize) -> Option<crate::Result<Vec<Vec<u8>>>> {
    let limit = A::USTKSIZE / 4;
    let mut buf = vec![0; limit];
    let mut args: Vec<Vec<u8>> = Vec::new();
    let mut strings = 0;
    loop {
        let at = argv.saturating_add(8 * args.len());
        if !user_range_ok::<A>(at, 8, Access::Read) {
            return None;
        }
        let mut p = [0; 8];
        copy_from_user::<A>(&mut p, at);
        let s = u64::from_le_bytes(p) as usize;
        if s == 0 {
            return Some(Ok(args));
        }
        let n = try_copy_str_from_user::<A>(&mut buf, s)?;
        strings += n + 1;
        if n == limit || stack_size(strings, args.len() + 1) > limit {
            return Some(Err(ENOVMEM));
        }
        args.push(buf[..n].to_vec());
    }
}

/// Close the current process's descriptors opened OCEXEC, as sysexec's
/// fdclose of each CCEXEC channel, dropping them once the table's unlocked.
fn close_cexec<A: ProcArch>() {
    let up = A::up();
    assert!(!up.is_null(), "exec: no current process");
    // SAFETY: up is the running process, and only its own syscalls, one at a
    // time, replace its table.
    let closed: [Option<Arc<Chan>>; NOPEN] =
        unsafe { (*up).fds() }.with(|fds| core::array::from_fn(|i| fds[i].take_if(|c| c.cexec())));
    drop(closed);
}

/// Make `pmmu` the current process's address space, with no segments, and
/// release the old one and every page in it, which the process owns, as
/// nothing shares a segment yet.
fn replace_pmmu<A: ProcArch>(pmmu: A::Pmmu) {
    let up = A::up();
    assert!(!up.is_null(), "exec: no current process");
    // SAFETY: up is the running process, and only its own syscalls, one at a
    // time, change its address space and segments.  Nothing refers to memory
    // through the old address space: what exec needs from it is copied in.
    unsafe {
        let old = core::mem::replace(&mut (*up).pmmu, pmmu);
        (*up).segments.clear();
        (*up).bss = None;
        A::switch(&(*up).pmmu);
        A::release(old, |pa| free_physpage(pa).expect("exec: couldn't free a process page"));
    }
}

/// Load the ELF executable `image` into the current process, map its stack,
/// and push `argv` onto it, as Plan 9's sysexec does, and set the process's
/// text to the last element of `name`.  Return the registers to enter it
/// with.  The arguments are checked first, as in sysexec, so bad ones map
/// nothing.  As with load, a failure after that isn't undone: the caller
/// must exit the process.
pub fn exec<A: ProcArch>(image: &[u8], name: &str, argv: &[&[u8]]) -> crate::Result<UserRegs> {
    args_size::<A>(argv)?;
    let (pc, mut segments) = load::<A>(image)?;
    map_zeroed::<A>(stack::<A>()).map_err(|err| match err {
        PutError::AlreadyMapped => EBADEXEC,
        PutError::NoMemory => ENOVMEM,
    })?;
    let regs = push_args::<A>(pc, A::USTKTOP, argv)?;
    segments.push(UserSegment { range: stack::<A>(), perm: Perm::Data });
    set_segments::<A>(segments);
    set_text::<A>(name);
    Ok(regs)
}

/// Set the current process's text to the last element of `name`, as Plan
/// 9's sysexec sets up->text to elem.
fn set_text<A: ProcArch>(name: &str) {
    let elem = name.rsplit('/').next().unwrap_or_default();
    let up = A::up();
    assert!(!up.is_null(), "exec: no current process");
    // SAFETY: up is the running process, and only its own syscalls, one at a
    // time, change its text.
    unsafe { (*up).text = FixedString::with_truncation_suffix(elem, "...") };
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
    if stack_size(strings, argv.len()) > A::USTKSIZE / 4 {
        return Err(ENOVMEM);
    }
    Ok(strings)
}

/// The stack `nargs` arguments take, given their strings' size, NULs
/// included: the strings, their pointers and the null after them, and 8
/// bytes of padding, the most a 16-byte aligned top leaves below them.
fn stack_size(strings: usize, nargs: usize) -> usize {
    strings.next_multiple_of(8).saturating_add(8 * (nargs + 1) + 8)
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
/// segments, set its bss to what follows its data, and return its entry
/// point and the segments its pages make, for the caller to record.
/// The layout is checked before anything is mapped.  A failure while mapping
/// isn't undone, as there's no going back after Plan 9's point of no return:
/// the caller must exit the process, whose release frees what was mapped.
pub fn load<A: ProcArch>(image: &[u8]) -> crate::Result<(usize, Vec<UserSegment>)> {
    let elf = elf::parse(image).map_err(|_| EBADEXEC)?;
    let bss = layout::<A>(&elf)?;
    for seg in elf.segments() {
        load_segment::<A>(&seg)?;
    }
    set_bss::<A>(bss);
    Ok((elf.entry(), segments(&elf, bss)))
}

/// The user stack's pages.
fn stack<A: MMU>() -> VirtRange {
    VirtRange::new(A::USTKTOP - A::USTKSIZE, A::USTKTOP)
}

/// The segments `elf`'s pages make, given its `bss`, as Plan 9's TSEG and
/// DSEG: each loadable segment's pages, but the data's only up to the bss's
/// start, rounded up, since the bss owns the pages from there, which brk_
/// frees when it shrinks.
fn segments(elf: &Elf, bss: VirtRange) -> Vec<UserSegment> {
    elf.segments()
        .map(|seg| {
            let start = seg.vaddr & !(PAGE_SIZE_4K - 1);
            let end = match seg.perm {
                Perm::Data => bss.start.next_multiple_of(PAGE_SIZE_4K),
                _ => (seg.vaddr + seg.memsz).next_multiple_of(PAGE_SIZE_4K),
            };
            UserSegment { range: VirtRange::new(start, end), perm: seg.perm }
        })
        .filter(|s| s.range.start < s.range.end)
        .collect()
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
    use crate::dat::ProcArch;
    use crate::fake::{Fake, USTKTOP, mapped, user_mem};

    /// The ELF loader's test program, built by xtask (user/elftest).
    static ELFTEST: &[u8] = include_bytes!(concat!(env!("R9_USER_BIN"), "/elftest"));

    /// ELFTEST with program header `i`'s field at `off` replaced by `value`.
    fn patched(i: usize, off: usize, value: u64) -> Vec<u8> {
        let mut image = ELFTEST.to_vec();
        let at = 64 + i * 56 + off;
        image[at..at + 8].copy_from_slice(&value.to_le_bytes());
        image
    }

    fn layout_of(image: &[u8]) -> crate::Result<VirtRange> {
        layout::<Fake>(&elf::parse(image).expect("didn't parse"))
    }

    #[test]
    fn elftest_layout() {
        // The bss starts after the data's file bytes, and ends on a page.
        let elf = elf::parse(ELFTEST).expect("didn't parse");
        let data = elf.segments().find(|s| s.perm == Perm::Data).expect("no data");
        let end = (data.vaddr + data.memsz).next_multiple_of(PAGE_SIZE_4K);
        let bss = VirtRange::new(data.vaddr + data.file.len(), end);
        assert_eq!(layout_of(ELFTEST), Ok(bss));
    }

    #[test]
    fn bad_layouts() {
        // Text moved down a page from UTZERO, and its entry with it.
        let entry = u64::from_le_bytes(ELFTEST[24..32].try_into().unwrap());
        let mut low = patched(0, 16, 0xf000);
        low[24..32].copy_from_slice(&(entry - 0x1000).to_le_bytes());
        assert_eq!(layout_of(&low), Err(EBADEXEC), "text below UTZERO");
        let guard = (USTKTOP - <Fake as MMU>::USTKSIZE - PAGE_SIZE_4K) as u64;
        assert_eq!(layout_of(&patched(2, 16, guard)), Err(EBADEXEC), "data in the guard page");
        // Rodata made writable: a second data segment.
        let mut two = ELFTEST.to_vec();
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
    fn elftest_segments() {
        let elf = elf::parse(ELFTEST).expect("didn't parse");
        let bss = layout_of(ELFTEST).expect("a layout");
        let segs = segments(&elf, bss);
        let page = |a: usize| a & !(PAGE_SIZE_4K - 1);
        let pgend = |a: usize| a.next_multiple_of(PAGE_SIZE_4K);

        // One for each loadable segment, in order, with its perm.
        let perms: Vec<Perm> = elf.segments().map(|s| s.perm).collect();
        assert_eq!(segs.iter().map(|s| s.perm).collect::<Vec<_>>(), perms);

        // The text and rodata are their whole pages; the data, the pages up
        // to the bss's, which start with the page after its last file byte.
        for (seg, s) in elf.segments().zip(&segs) {
            let end = match seg.perm {
                Perm::Data => pgend(seg.vaddr + seg.file.len()),
                _ => pgend(seg.vaddr + seg.memsz),
            };
            assert_eq!(s.range, VirtRange::new(page(seg.vaddr), end), "{:?}", seg.perm);
        }

        // Together with the bss's pages, they're the pages load maps, each
        // once.
        let mut pages: Vec<usize> =
            segs.iter().flat_map(|s| (s.range.start..s.range.end).step_by(PAGE_SIZE_4K)).collect();
        pages.extend((pgend(bss.start)..bss.end).step_by(PAGE_SIZE_4K));
        let mut loaded: Vec<usize> = elf
            .segments()
            .flat_map(|s| (page(s.vaddr)..pgend(s.vaddr + s.memsz)).step_by(PAGE_SIZE_4K))
            .collect();
        pages.sort();
        loaded.sort();
        assert_eq!(pages, loaded);
    }

    #[test]
    fn exec_checks_args_before_loading() {
        // A bad argument is refused before load, which would fail for want of
        // memory on the host.
        assert_eq!(exec::<Fake>(ELFTEST, "elftest", &[b"a\0b"]), Err(EBADARG));
        assert!(mapped().is_empty());
    }

    /// A device whose one file is `LEN` bytes of 'x'.
    struct Big;
    const LEN: u64 = 100;

    impl crate::dev::Dev for Big {
        fn dc(&self) -> char {
            'B'
        }
        fn attach(&self, _spec: &str) -> crate::Result<Arc<Chan>> {
            unreachable!()
        }
        fn walk(&self, _c: &Arc<Chan>, _elems: &[&str]) -> crate::Result<Arc<Chan>> {
            unreachable!()
        }
        fn open(&self, _c: &Arc<Chan>, _omode: u32) -> crate::Result<Arc<Chan>> {
            unreachable!()
        }
        fn read(&self, _c: &Arc<Chan>, buf: &mut [u8], offset: u64) -> crate::Result<u64> {
            let n = (LEN.saturating_sub(offset) as usize).min(buf.len()).min(7);
            buf[..n].fill(b'x');
            Ok(n as u64)
        }
        fn write(&self, _c: &Arc<Chan>, _buf: &[u8], _offset: u64) -> crate::Result<u64> {
            unreachable!()
        }
        fn close(&self, _c: &Chan) {}
    }

    #[test]
    fn read_image_reads_whole_but_bounded() {
        let qid = crate::chan::Qid { path: 0, kind: 0 };
        let c = Chan::opened(&Big, qid, 0, LEN);
        let image = read_image(&c, 100).expect("the image");
        assert_eq!(image, vec![b'x'; 100], "short reads, to the end");
        assert_eq!(image.capacity(), 101, "sized by the file's length");
        assert_eq!(read_image(&c, 99), Err(EBADEXEC), "a byte too many");
        // A file longer than its length, or without one, is read anyway.
        for length in [0, 10] {
            let c = Chan::opened(&Big, qid, 0, length);
            assert_eq!(read_image(&c, 100), Ok(vec![b'x'; 100]), "a length of {length}");
        }
    }

    #[test]
    fn copy_argv_copies_or_reports_a_bad_address() {
        let mut mem = user_mem();
        let base = mem.base();
        let unmapped = base + 2 * PAGE_SIZE_4K;
        let put = |mem: &mut crate::fake::Mem, at: usize, p: usize| {
            mem.0[at..at + 8].copy_from_slice(&(p as u64).to_le_bytes())
        };
        mem.0[256..259].copy_from_slice(b"ab\0");
        mem.0[260..261].copy_from_slice(b"\0");
        put(&mut mem, 0, base + 256);
        put(&mut mem, 8, base + 260);
        put(&mut mem, 16, 0);
        assert_eq!(copy_argv::<Fake>(base), Some(Ok(vec![b"ab".to_vec(), vec![]])));
        assert_eq!(copy_argv::<Fake>(unmapped), None, "an unmapped array");
        put(&mut mem, 8, unmapped);
        assert_eq!(copy_argv::<Fake>(base), None, "an unmapped string");
        // A string that runs off its page into the unmapped one.
        mem.0[PAGE_SIZE_4K..2 * PAGE_SIZE_4K].fill(b'x');
        put(&mut mem, 8, base + PAGE_SIZE_4K);
        assert_eq!(copy_argv::<Fake>(base), None, "a string reaching an unmapped page");
        // Too many: the same string, a quarter of the stack.
        mem.0[2 * PAGE_SIZE_4K - 1] = 0;
        for i in 0..5 {
            put(&mut mem, 8 * i, base + PAGE_SIZE_4K);
        }
        put(&mut mem, 40, 0);
        assert_eq!(copy_argv::<Fake>(base), Some(Err(ENOVMEM)), "too many");
    }

    #[test]
    fn close_cexec_closes_only_ocexec() {
        use crate::chan::{OCEXEC, OWRITE};
        let mut p = crate::proc::newproc::<Fake>(|| {}).expect("newproc");
        Fake::set_up(&mut *p);
        let cexec = open_name("#c/cons", OWRITE | OCEXEC).expect("an OCEXEC open");
        assert!(cexec.cexec(), "OCEXEC is recorded");
        p.fds().with(|fds| {
            fds[3] = Some(cexec.clone());
            fds[4] = fds[1].clone();
            fds[5] = Some(cexec);
        });
        close_cexec::<Fake>();
        let open: Vec<bool> = p.fds().with(|fds| fds.iter().map(Option::is_some).collect());
        assert_eq!(&open[..6], [true, true, true, false, true, false], "both of OCEXEC's dups");
        Fake::set_up(core::ptr::null_mut());
        // SAFETY: nothing runs in the fake process.
        unsafe { crate::proc::reap(*p) };
    }

    #[test]
    fn load_maps_nothing_without_memory() {
        // The host page allocator has nothing free.
        assert_eq!(load::<Fake>(ELFTEST), Err(ENOVMEM));
        assert!(mapped().is_empty());
    }
}
