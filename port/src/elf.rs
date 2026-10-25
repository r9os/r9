//! ELF64 executables: validating a static aarch64 ET_EXEC image and reading
//! its loadable segments, without allocating.  Offsets and values are from the
//! System V gABI's ELF-64 object file format.  The image is untrusted, so every
//! read is bounds-checked and every sum overflow-checked.  Where segments may
//! go in memory is the loader's business, not the parser's.

use crate::dat::Perm;
use crate::mem::PAGE_SIZE_4K;

const EHDR_SIZE: usize = 64;
const PHDR_SIZE: usize = 56;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_AARCH64: u16 = 183;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

/// Why an image isn't a loadable executable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ElfError {
    /// Shorter than an ELF header.
    Truncated,
    BadMagic,
    NotElf64,
    NotLittleEndian,
    BadVersion,
    /// Not ET_EXEC: shared objects and PIEs aren't supported.
    NotExecutable,
    NotAarch64,
    BadPhentsize,
    /// The program headers aren't all within the image.
    PhdrsOutOfRange,
    /// A segment's file bytes aren't all within the image.
    SegmentOutOfRange,
    FileszOverMemsz,
    /// A segment's address and file offset differ within a page.
    Misaligned,
    /// A segment ends past the top of the address space.
    Overflow,
    /// A segment is writable and executable, or isn't readable.
    BadFlags,
    /// Segments aren't in ascending order, or two share a page.
    Unordered,
    /// The entry point isn't in a text segment.
    EntryNotInText,
    /// The entry point isn't on an instruction boundary.
    MisalignedEntry,
}

/// A validated executable image.
pub struct Elf<'a> {
    image: &'a [u8],
    phdrs: &'a [[u8; PHDR_SIZE]],
    entry: usize,
}

/// A loadable segment: `memsz` bytes at `vaddr`, the first of which are
/// `file`, and the rest zero.
#[derive(Debug, PartialEq)]
pub struct Segment<'a> {
    pub vaddr: usize,
    pub memsz: usize,
    pub file: &'a [u8],
    pub perm: Perm,
}

impl<'a> Elf<'a> {
    pub fn entry(&self) -> usize {
        self.entry
    }

    /// The loadable segments, in ascending order, no two sharing a page.
    pub fn segments(&self) -> impl Iterator<Item = Segment<'a>> + use<'a> {
        let image = self.image;
        // parse has validated every segment, so none is dropped here.
        self.phdrs.iter().filter_map(move |ph| segment(image, ph).ok().flatten())
    }
}

/// Validate `image` as a static aarch64 executable.
pub fn parse(image: &[u8]) -> Result<Elf<'_>, ElfError> {
    let h = image.first_chunk::<EHDR_SIZE>().ok_or(ElfError::Truncated)?;
    if h[..4] != *b"\x7fELF" {
        return Err(ElfError::BadMagic);
    }
    if h[4] != ELFCLASS64 {
        return Err(ElfError::NotElf64);
    }
    if h[5] != ELFDATA2LSB {
        return Err(ElfError::NotLittleEndian);
    }
    if h[6] != EV_CURRENT || u32_at(h, 20) != EV_CURRENT as u32 {
        return Err(ElfError::BadVersion);
    }
    if u16_at(h, 16) != ET_EXEC {
        return Err(ElfError::NotExecutable);
    }
    if u16_at(h, 18) != EM_AARCH64 {
        return Err(ElfError::NotAarch64);
    }
    if u16_at(h, 54) as usize != PHDR_SIZE {
        return Err(ElfError::BadPhentsize);
    }
    let entry = u64_at(h, 24) as usize;
    // A64 instructions are 4 bytes, and a misaligned pc faults.
    if !entry.is_multiple_of(4) {
        return Err(ElfError::MisalignedEntry);
    }
    let phoff = u64_at(h, 32) as usize;
    let phnum = u16_at(h, 56) as usize;
    let phdrs = phoff
        .checked_add(phnum * PHDR_SIZE)
        .and_then(|end| image.get(phoff..end))
        .ok_or(ElfError::PhdrsOutOfRange)?
        .as_chunks::<PHDR_SIZE>()
        .0;

    let mut prev_end = 0;
    let mut entry_in_text = false;
    for ph in phdrs {
        let Some(seg) = segment(image, ph)? else {
            continue;
        };
        // segment checked that this can't overflow.
        let end = (seg.vaddr + seg.memsz).next_multiple_of(PAGE_SIZE_4K);
        if seg.vaddr & !(PAGE_SIZE_4K - 1) < prev_end {
            return Err(ElfError::Unordered);
        }
        prev_end = end;
        let file_end = seg.vaddr + seg.file.len();
        entry_in_text |= seg.perm == Perm::Text && (seg.vaddr..file_end).contains(&entry);
    }
    if !entry_in_text {
        return Err(ElfError::EntryNotInText);
    }
    Ok(Elf { image, phdrs, entry })
}

/// The segment that program header `ph` describes, or None if it isn't a
/// non-empty PT_LOAD.
fn segment<'a>(image: &'a [u8], ph: &[u8; PHDR_SIZE]) -> Result<Option<Segment<'a>>, ElfError> {
    let memsz = u64_at(ph, 40) as usize;
    if u32_at(ph, 0) != PT_LOAD || memsz == 0 {
        return Ok(None);
    }
    let flags = u32_at(ph, 4);
    let offset = u64_at(ph, 8) as usize;
    let vaddr = u64_at(ph, 16) as usize;
    let filesz = u64_at(ph, 32) as usize;
    if filesz > memsz {
        return Err(ElfError::FileszOverMemsz);
    }
    let file = offset
        .checked_add(filesz)
        .and_then(|end| image.get(offset..end))
        .ok_or(ElfError::SegmentOutOfRange)?;
    if vaddr % PAGE_SIZE_4K != offset % PAGE_SIZE_4K {
        return Err(ElfError::Misaligned);
    }
    if vaddr.checked_add(memsz).and_then(|end| end.checked_next_multiple_of(PAGE_SIZE_4K)).is_none()
    {
        return Err(ElfError::Overflow);
    }
    let perm = match flags & (PF_R | PF_W | PF_X) {
        f if f == PF_R | PF_X => Perm::Text,
        f if f == PF_R => Perm::Rodata,
        f if f == PF_R | PF_W => Perm::Data,
        _ => return Err(ElfError::BadFlags),
    };
    Ok(Some(Segment { vaddr, memsz, file, perm }))
}

// Callers pass the header or a program header, which are arrays long enough for
// every field read from them.
fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    static HELLO: &[u8] = include_bytes!("../lib/test/elf/hello.elf");

    /// The offset of program header `i`'s field at `off`.
    fn ph(i: usize, off: usize) -> usize {
        u64_at(HELLO, 32) as usize + i * PHDR_SIZE + off
    }

    /// HELLO with the bytes at `off` replaced by `bytes`.
    fn patched(off: usize, bytes: &[u8]) -> Vec<u8> {
        let mut image = HELLO.to_vec();
        image[off..off + bytes.len()].copy_from_slice(bytes);
        image
    }

    fn err(image: &[u8]) -> ElfError {
        parse(image).err().expect("parsed")
    }

    #[test]
    fn hello_parses() {
        let elf = parse(HELLO).expect("hello.elf didn't parse");
        assert_eq!(elf.entry(), 0x10000);
        let segs: Vec<_> =
            elf.segments().map(|s| (s.vaddr, s.memsz, s.file.len(), s.perm)).collect();
        assert_eq!(
            segs,
            [
                (0x10000, 156, 156, Perm::Text),
                (0x11000, 15, 15, Perm::Rodata),
                (0x12000, 9224, 4104, Perm::Data)
            ]
        );
        let rodata = elf.segments().nth(1).unwrap();
        assert_eq!(rodata.file, b"hello from elf\n");
    }

    #[test]
    fn bad_headers() {
        assert_eq!(err(&HELLO[..EHDR_SIZE - 1]), ElfError::Truncated);
        assert_eq!(err(&patched(0, b"\x7fELG")), ElfError::BadMagic);
        assert_eq!(err(&patched(4, &[1])), ElfError::NotElf64);
        assert_eq!(err(&patched(5, &[2])), ElfError::NotLittleEndian);
        assert_eq!(err(&patched(6, &[0])), ElfError::BadVersion);
        assert_eq!(err(&patched(20, &[2])), ElfError::BadVersion);
        assert_eq!(err(&patched(16, &3u16.to_le_bytes())), ElfError::NotExecutable, "ET_DYN");
        assert_eq!(err(&patched(18, &62u16.to_le_bytes())), ElfError::NotAarch64, "x86-64");
        assert_eq!(err(&patched(54, &55u16.to_le_bytes())), ElfError::BadPhentsize);
        assert_eq!(err(&patched(32, &u64::MAX.to_le_bytes())), ElfError::PhdrsOutOfRange);
        assert_eq!(err(&HELLO[..ph(2, 0)]), ElfError::PhdrsOutOfRange, "truncated phdrs");
        let entry = 0x11000u64.to_le_bytes();
        assert_eq!(err(&patched(24, &entry)), ElfError::EntryNotInText, "entry in rodata");
        let entry = 0x10002u64.to_le_bytes();
        assert_eq!(err(&patched(24, &entry)), ElfError::MisalignedEntry);
    }

    #[test]
    fn bad_segments() {
        let big = u64::MAX.to_le_bytes();
        assert_eq!(err(&patched(ph(2, 8), &big)), ElfError::SegmentOutOfRange, "offset");
        assert_eq!(err(&HELLO[..HELLO.len() - 3000]), ElfError::SegmentOutOfRange, "truncated");
        assert_eq!(err(&patched(ph(2, 32), &10000u64.to_le_bytes())), ElfError::FileszOverMemsz);
        assert_eq!(err(&patched(ph(1, 16), &0x11008u64.to_le_bytes())), ElfError::Misaligned);
        let top = (u64::MAX & !0xfff).to_le_bytes();
        assert_eq!(err(&patched(ph(2, 16), &top)), ElfError::Overflow);
        let rwx = (PF_R | PF_W | PF_X).to_le_bytes();
        assert_eq!(err(&patched(ph(0, 4), &rwx)), ElfError::BadFlags, "W+X");
        assert_eq!(err(&patched(ph(0, 4), &PF_X.to_le_bytes())), ElfError::BadFlags, "X only");
        let shared = 0x10000u64.to_le_bytes();
        assert_eq!(err(&patched(ph(1, 16), &shared)), ElfError::Unordered, "sharing a page");
        let below = 0xf000u64.to_le_bytes();
        assert_eq!(err(&patched(ph(1, 16), &below)), ElfError::Unordered, "descending");
    }

    #[test]
    fn other_segments_are_ignored() {
        // PT_NOTE
        let image = patched(ph(1, 0), &4u32.to_le_bytes());
        let elf = parse(&image).expect("didn't parse");
        assert_eq!(elf.segments().count(), 2);
    }
}
