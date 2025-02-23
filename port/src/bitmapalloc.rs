/// bitmapalloc implements a very simple bitmap page allocator.
///
/// Benefits of the current implementation:
///  - Doesn't require any allocations, so can be used without fear while
///    manipulating the page tables.
///
/// Downsides:
///  - Can't be dynamically resized.
use core::fmt;

use crate::mem::{PhysAddr, PhysRange};

/// Simple bitmap.  Bear in mind that logically, bit 0 is the rightmost bit,
/// so writing out as bytes will have the bits logically reversed.
struct Bitmap<const SIZE_BYTES: usize> {
    bytes: [u8; SIZE_BYTES],
}

impl<const SIZE_BYTES: usize> Bitmap<SIZE_BYTES> {
    pub const fn new(init_value: u8) -> Self {
        Self { bytes: [init_value; SIZE_BYTES] }
    }

    /// Is bit `i` within the bitmap set?
    pub fn is_set(&self, i: usize) -> bool {
        let byte_idx = i / 8;
        let bit_idx = i % 8;
        let byte = self.bytes[byte_idx];
        byte & (1 << bit_idx) > 0
    }

    /// Set bit `i` within the bitmap
    pub fn set(&mut self, i: usize, b: bool) {
        let byte_idx = i / 8;
        let bit_idx = i % 8;
        if b {
            self.bytes[byte_idx] |= 1 << bit_idx;
        } else {
            self.bytes[byte_idx] &= !(1 << bit_idx);
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum BitmapPageAllocError {
    NotEnoughBitmaps,
    OutOfBounds,
    MisalignedAddr,
    OutOfSpace,
    NotAllocated,
}

/// Allocator where each page is represented by a single bit.
///   0: free, 1: allocated
/// `end` is used to indicate the extent of the memory.  Anything beyond this
/// will be marked as allocated.
pub struct BitmapPageAlloc<const NUM_BITMAPS: usize, const BITMAP_SIZE_BYTES: usize> {
    bitmaps: [Bitmap<BITMAP_SIZE_BYTES>; NUM_BITMAPS],
    alloc_page_size: usize,    // Size of pages represented by single bit
    end: PhysAddr,             // Upper bound of physical memory
    next_pa_to_scan: PhysAddr, // PhysAddr from which to start scanning for next allocation
}

impl<const NUM_BITMAPS: usize, const BITMAP_SIZE_BYTES: usize>
    BitmapPageAlloc<NUM_BITMAPS, BITMAP_SIZE_BYTES>
{
    pub const fn new_all_allocated(alloc_page_size: usize) -> Self {
        let end = PhysAddr::new((NUM_BITMAPS * BITMAP_SIZE_BYTES * 8 * alloc_page_size) as u64);
        Self {
            bitmaps: [const { Bitmap::<BITMAP_SIZE_BYTES>::new(0xff) }; NUM_BITMAPS],
            alloc_page_size,
            end,
            next_pa_to_scan: PhysAddr::new(0),
        }
    }

    /// Returns number of physical bytes a single bitmap can cover.
    const fn bytes_per_bitmap_byte(&self) -> usize {
        8 * self.alloc_page_size
    }

    /// Returns number of physical bytes a single bitmap can cover.
    const fn bytes_per_bitmap(&self) -> usize {
        BITMAP_SIZE_BYTES * self.bytes_per_bitmap_byte()
    }

    /// Returns number of physical bytes covered by all bitmaps.
    const fn max_bytes(&self) -> usize {
        NUM_BITMAPS * self.bytes_per_bitmap()
    }

    /// Mark the bits corresponding to the given physical range as allocated,
    /// regardless of the existing state.
    pub fn mark_allocated(&mut self, range: &PhysRange) -> Result<(), BitmapPageAllocError> {
        self.mark_range(range, true, true)
    }

    /// Mark the bits corresponding to the given physical range as free,
    /// regardless of the existing state.
    pub fn mark_free(&mut self, range: &PhysRange) -> Result<(), BitmapPageAllocError> {
        self.mark_range(range, false, true)
    }

    /// Free unused pages in mem that aren't covered by the memory map.  Assumes
    /// that custom_map is sorted and that available_mem can be used to set the
    /// upper bound of the allocator.
    pub fn free_unused_ranges<'a>(
        &mut self,
        available_mem: &PhysRange,
        used_ranges: impl Iterator<Item = &'a PhysRange>,
    ) -> Result<(), BitmapPageAllocError> {
        let mut next_start = available_mem.start();
        for range in used_ranges {
            if next_start < range.0.start {
                self.mark_free(&PhysRange::new(next_start, range.0.start))?;
            }
            if next_start < range.0.end {
                next_start = range.0.end;
            }
        }
        if next_start < available_mem.end() {
            self.mark_free(&PhysRange::new(next_start, available_mem.end()))?;
        }

        self.end = available_mem.0.end;

        // Mark everything past the end point as allocated
        let end_range = PhysRange::new(self.end, PhysAddr::new(self.max_bytes() as u64));
        self.mark_range(&end_range, true, false)?;

        self.next_pa_to_scan = PhysAddr::new(0); // Just set to 0 for simplicity - could be smarter

        Ok(())
    }

    /// Try to allocate the next available page.
    pub fn allocate(&mut self) -> Result<PhysAddr, BitmapPageAllocError> {
        let (first_bitmap_idx, first_byte_idx, _) = self.physaddr_as_indices(self.next_pa_to_scan);

        let found_indices = self
            .indices_from(first_bitmap_idx, first_byte_idx)
            .find(|indices| self.byte(indices) != 0xff);

        if let Some(indices) = found_indices {
            // Mark the page as allocated and return the address
            let byte = &mut self.bitmaps[indices.bitmap].bytes[indices.byte];
            let num_leading_ones = byte.trailing_ones() as usize;
            *byte |= 1 << num_leading_ones;

            let pa = self.indices_as_physaddr(indices.bitmap, indices.byte, num_leading_ones);
            self.next_pa_to_scan = pa;
            Ok(pa)
        } else {
            Err(BitmapPageAllocError::OutOfSpace)
        }
    }

    /// Deallocate the page corresponding to the given PhysAddr.
    pub fn deallocate(&mut self, pa: PhysAddr) -> Result<(), BitmapPageAllocError> {
        if pa > self.end {
            return Err(BitmapPageAllocError::OutOfBounds);
        }

        let (bitmap_idx, byte_idx, bit_idx) = self.physaddr_as_indices(pa);

        let bitmap = &mut self.bitmaps[bitmap_idx];
        if !bitmap.is_set(8 * byte_idx + bit_idx) {
            return Err(BitmapPageAllocError::NotAllocated);
        }
        bitmap.set(bit_idx, false);

        self.next_pa_to_scan = pa; // Next allocation will reuse this

        Ok(())
    }

    /// Return a tuple of (bytes used, total bytes available) based on the page allocator.
    pub fn usage_bytes(&self) -> (usize, usize) {
        // We count free because the last bits might be marked partially 'allocated'
        // if the end comes in the middle of a byte in the bitmap.
        let mut free_bytes: usize = 0;
        for indices in self.indices() {
            free_bytes += self.byte(&indices).count_zeros() as usize * self.alloc_page_size;
        }
        let total = self.end.0 as usize;
        (total - free_bytes, total)
    }

    /// For the given physaddr, returns a tuple of (the bitmap containing pa,
    /// the index of the byte containing the pa, and the index of the bit within that byte).
    fn physaddr_as_indices(&self, pa: PhysAddr) -> (usize, usize, usize) {
        assert_eq!(pa.addr() % self.alloc_page_size as u64, 0);

        // Get the index of the bitmap containing the pa
        let bytes_per_bitmap = self.bytes_per_bitmap();
        let bitmap_idx = pa.addr() as usize / bytes_per_bitmap;

        // Get the byte within the bitmap representing the pa
        let pa_offset_into_bitmap = pa.addr() as usize % bytes_per_bitmap;
        let bytes_per_bitmap_byte = self.bytes_per_bitmap_byte();
        let byte_idx = pa_offset_into_bitmap / bytes_per_bitmap_byte;

        // Finally get the bit within the byte
        let bit_idx =
            (pa_offset_into_bitmap - (byte_idx * bytes_per_bitmap_byte)) / self.alloc_page_size;

        (bitmap_idx, byte_idx, bit_idx)
    }

    /// Given the bitmap index, byte index within the bitmap, and bit index within the byte,
    /// return the corresponding PhysAddr.
    fn indices_as_physaddr(&self, bitmap_idx: usize, byte_idx: usize, bit_idx: usize) -> PhysAddr {
        PhysAddr::new(
            ((bitmap_idx * self.bytes_per_bitmap())
                + (byte_idx * self.bytes_per_bitmap_byte())
                + (bit_idx * self.alloc_page_size)) as u64,
        )
    }

    fn mark_range(
        &mut self,
        range: &PhysRange,
        mark_allocated: bool,
        check_end: bool,
    ) -> Result<(), BitmapPageAllocError> {
        if check_end && range.0.end > self.end {
            return Err(BitmapPageAllocError::NotEnoughBitmaps);
        }

        for pa in range.step_by_rounded(self.alloc_page_size) {
            let (bitmap_idx, byte_idx, bit_idx) = self.physaddr_as_indices(pa);
            if bitmap_idx >= self.bitmaps.len() {
                return Err(BitmapPageAllocError::OutOfBounds);
            }

            let bitmap = &mut self.bitmaps[bitmap_idx];
            bitmap.set(8 * byte_idx + bit_idx, mark_allocated);
        }
        Ok(())
    }

    /// Iterate over each of the bytes in turn.  Iterates only over the bytes
    /// covering pages up to `end`.  If `end` is within one of the bytes, that
    /// byte will be returned.
    fn indices(&self) -> impl Iterator<Item = ByteIndices> + '_ {
        self.indices_from(0, 0)
    }

    /// Iterate over each of the bytes in turn, starting from a particular bitmap
    /// and byte, and looping to iterate across all bytes.  Iterates only over the bytes
    /// covering pages up to `end`.  If `end` is within one of the bytes, that
    /// byte will be returned.
    fn indices_from(
        &self,
        start_bitmap_idx: usize,
        start_byte_idx: usize,
    ) -> impl Iterator<Item = ByteIndices> + '_ {
        let mut bitmap_idx = start_bitmap_idx;
        let mut byte_idx = start_byte_idx;
        let mut passed_first = false;
        let mut currpa = self.indices_as_physaddr(bitmap_idx, byte_idx, 0);

        core::iter::from_fn(move || {
            // Catch when we've iterated to the end of the last bitmap and need to
            // cycle back to the start
            if bitmap_idx >= self.bitmaps.len() || currpa >= self.end {
                bitmap_idx = 0;
                byte_idx = 0;
                currpa = PhysAddr::new(0);
            }

            // Catch when we've iterated over all the bytes
            if passed_first && bitmap_idx == start_bitmap_idx && byte_idx == start_byte_idx {
                return None;
            }
            passed_first = true;

            // Return the byte and prepare for the next
            let indices = ByteIndices { bitmap: bitmap_idx, byte: byte_idx };
            byte_idx += 1;
            if byte_idx >= BITMAP_SIZE_BYTES {
                byte_idx = 0;
                bitmap_idx += 1;
                currpa.0 += self.alloc_page_size as u64;
            }
            Some(indices)
        })
    }

    fn byte(&self, indices: &ByteIndices) -> u8 {
        self.bitmaps[indices.bitmap].bytes[indices.byte]
    }

    #[cfg(test)]
    fn bytes(&self) -> Vec<u8> {
        self.indices().map(|idx| self.byte(&idx)).collect::<Vec<u8>>()
    }

    #[cfg(test)]
    fn bytes_from(&self, start_bitmap_idx: usize, start_byte_idx: usize) -> Vec<u8> {
        self.indices_from(start_bitmap_idx, start_byte_idx)
            .map(|idx| self.byte(&idx))
            .collect::<Vec<u8>>()
    }
}

struct ByteIndices {
    bitmap: usize,
    byte: usize,
}

/// fmt::Debug is useful in small test cases, but would be too verbose for a
/// realistic bitmap.
impl<const NUM_BITMAPS: usize, const BITMAP_SIZE_BYTES: usize> fmt::Debug
    for BitmapPageAlloc<NUM_BITMAPS, BITMAP_SIZE_BYTES>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x")?;
        for b in self.indices() {
            write!(f, "{:02x}", self.byte(&b))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_new() {
        let bitmap = Bitmap::<4096>::new(0);
        for byte in bitmap.bytes {
            assert_eq!(byte, 0x00);
        }
    }

    #[test]
    fn bitmap_set() {
        let mut bitmap = Bitmap::<4096>::new(0);
        assert!(!bitmap.is_set(0));
        bitmap.set(0, true);
        assert!(bitmap.is_set(0));

        // Assert only this bit is set
        assert_eq!(bitmap.bytes[0], 1);
        for i in 1..bitmap.bytes.len() {
            assert_eq!(bitmap.bytes[i], 0);
        }
    }

    #[test]
    fn iterate() {
        let alloc = BitmapPageAlloc::<2, 2>::new_all_allocated(4);
        assert_eq!(alloc.bytes(), vec![255; 4]);
        assert_eq!(alloc.bytes_from(1, 0), vec![255; 4]);
    }

    #[test]
    fn bitmappagealloc_mark_allocated_and_free() -> Result<(), BitmapPageAllocError> {
        // Create a new allocator and mark it all freed
        // 2 bitmaps, 2 bytes per bitmap, mapped to pages of 4 bytes
        // 32 bits, 128 bytes physical memory
        let mut alloc = BitmapPageAlloc::<2, 2>::new_all_allocated(4);
        alloc.mark_free(&PhysRange::with_end(0, alloc.max_bytes() as u64))?;

        // Mark a range as allocated - 10 bits
        alloc.mark_allocated(&PhysRange::with_end(4, 44))?;
        assert_eq!(alloc.bytes(), [0xfe, 0x07, 0x00, 0x00]);

        // Deallocate a range - first 2 bits
        alloc.mark_free(&PhysRange::with_end(0, 8))?;
        assert_eq!(alloc.bytes(), [0xfc, 0x07, 0x00, 0x00]);
        Ok(())
    }

    #[test]
    fn bitmappagealloc_allocate_and_deallocate() -> Result<(), BitmapPageAllocError> {
        // Create a new allocator and mark it all freed
        // 2 bitmaps, 2 bytes per bitmap, mapped to pages of 4 bytes
        // 32 bits, 128 bytes physical memory
        let mut alloc = BitmapPageAlloc::<2, 2>::new_all_allocated(4);
        alloc.mark_free(&PhysRange::with_end(0, alloc.max_bytes() as u64))?;
        assert_eq!(alloc.usage_bytes(), (0, 128));

        // Mark a range as allocated - 10 bits
        alloc.mark_allocated(&PhysRange::with_end(4, 44))?;
        assert_eq!(alloc.usage_bytes(), (40, 128));
        assert_eq!(alloc.bytes(), [0xfe, 0x07, 0x00, 0x00]);

        // Now try to allocate the next 3 free pages
        assert_eq!(alloc.allocate()?, PhysAddr::new(0));
        assert_eq!(alloc.allocate()?, PhysAddr::new(44));
        assert_eq!(alloc.allocate()?, PhysAddr::new(48));

        // Allocate until we run out of pages.  At this point there are 19 pages left,
        // so allocate them, and then assert one more fails
        for _ in 0..19 {
            alloc.allocate()?;
        }
        assert_eq!(alloc.bytes(), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(alloc.allocate().unwrap_err(), BitmapPageAllocError::OutOfSpace);

        // Now try to deallocate the second page
        assert!(alloc.deallocate(PhysAddr::new(4)).is_ok());
        assert_eq!(alloc.bytes(), [0xfd, 0xff, 0xff, 0xff]);

        // Ensure double deallocation fails
        assert_eq!(
            alloc.deallocate(PhysAddr::new(4)).unwrap_err(),
            BitmapPageAllocError::NotAllocated
        );
        assert_eq!(alloc.bytes(), [0xfd, 0xff, 0xff, 0xff]);

        // Allocate once more, expecting the physical address we just deallocated
        assert_eq!(alloc.allocate()?, PhysAddr::new(4));

        Ok(())
    }

    #[test]
    fn physaddr_as_indices() {
        let alloc = BitmapPageAlloc::<2, 4096>::new_all_allocated(4096);
        let bytes_per_bitmap = alloc.bytes_per_bitmap() as u64;

        assert_eq!(alloc.physaddr_as_indices(PhysAddr::new(0)), (0, 0, 0));
        assert_eq!(alloc.physaddr_as_indices(PhysAddr::new(4096)), (0, 0, 1));
        assert_eq!(alloc.physaddr_as_indices(PhysAddr::new(8192)), (0, 0, 2));
        assert_eq!(alloc.physaddr_as_indices(PhysAddr::new(4096 * 8)), (0, 1, 0));
        assert_eq!(alloc.physaddr_as_indices(PhysAddr::new(4096 * 9)), (0, 1, 1));
        assert_eq!(alloc.physaddr_as_indices(PhysAddr::new(bytes_per_bitmap)), (1, 0, 0));
        assert_eq!(
            alloc.physaddr_as_indices(PhysAddr::new(bytes_per_bitmap + 4096 * 9)),
            (1, 1, 1)
        );
    }

    #[test]
    fn indices_as_physaddr() {
        let alloc = BitmapPageAlloc::<2, 4096>::new_all_allocated(4096);
        let bytes_per_bitmap = alloc.bytes_per_bitmap() as u64;

        assert_eq!(alloc.indices_as_physaddr(0, 0, 0), PhysAddr::new(0));
        assert_eq!(alloc.indices_as_physaddr(0, 0, 1), PhysAddr::new(4096));
        assert_eq!(alloc.indices_as_physaddr(0, 1, 0), PhysAddr::new(4096 * 8));
        assert_eq!(alloc.indices_as_physaddr(0, 1, 1), PhysAddr::new(4096 * 9));
        assert_eq!(alloc.indices_as_physaddr(1, 0, 0), PhysAddr::new(bytes_per_bitmap));
        assert_eq!(alloc.indices_as_physaddr(1, 1, 1), PhysAddr::new(bytes_per_bitmap + 4096 * 9));
    }
}
