use core::alloc::{AllocError, Allocator, Layout};
use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

#[cfg(not(test))]
use crate::println;

/// Bump allocator to be used for earliest allocations in r9.  These allocations
/// can never be freed - attempting to do so will panic.
#[repr(C, align(4096))]
pub struct Bump<const SIZE_BYTES: usize, const MAX_SUPPORTED_ALIGN: usize> {
    bytes: UnsafeCell<[u8; SIZE_BYTES]>,
    next_offset: AtomicUsize,
    wasted: AtomicUsize,
}

unsafe impl<const SIZE_BYTES: usize, const MAX_SUPPORTED_ALIGN: usize> Send
    for Bump<SIZE_BYTES, MAX_SUPPORTED_ALIGN>
{
}

unsafe impl<const SIZE_BYTES: usize, const MAX_SUPPORTED_ALIGN: usize> Sync
    for Bump<SIZE_BYTES, MAX_SUPPORTED_ALIGN>
{
}

impl<const SIZE_BYTES: usize, const MAX_SUPPORTED_ALIGN: usize>
    Bump<SIZE_BYTES, MAX_SUPPORTED_ALIGN>
{
    pub const fn new(init_value: u8) -> Self {
        Self {
            bytes: UnsafeCell::new([init_value; SIZE_BYTES]),
            next_offset: AtomicUsize::new(0),
            wasted: AtomicUsize::new(0),
        }
    }

    pub fn print_status(&self) {
        let allocated = self.next_offset.load(Relaxed);
        let remaining = SIZE_BYTES - allocated;
        let wasted = self.wasted.load(Relaxed);
        println!(
            "Bump: allocated: {allocated} free: {remaining} total: {SIZE_BYTES} wasted: {wasted}"
        );
    }

    /// Test helper to get the offset of the result in the buffer
    #[cfg(test)]
    fn result_offset(&self, result: Result<NonNull<[u8]>, AllocError>) -> Option<isize> {
        unsafe {
            result
                .ok()
                .map(|bytes| bytes.byte_offset_from(NonNull::new_unchecked(self.bytes.get())))
        }
    }
}

unsafe impl<const SIZE_BYTES: usize, const MAX_SUPPORTED_ALIGN: usize> Allocator
    for Bump<SIZE_BYTES, MAX_SUPPORTED_ALIGN>
{
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let align = layout.align();

        if align > MAX_SUPPORTED_ALIGN {
            return Err(AllocError {});
        }

        let mut wasted = 0;
        let mut alloc_offset = 0;
        let alloc_ok = self
            .next_offset
            .fetch_update(Relaxed, Relaxed, |last_offset| {
                let align_mask = !(align - 1);
                alloc_offset = if last_offset & !align_mask != 0 {
                    (last_offset + align) & align_mask
                } else {
                    last_offset
                };
                wasted = alloc_offset - last_offset;

                let new_offset = alloc_offset + size;
                if new_offset > SIZE_BYTES {
                    None
                } else {
                    Some(new_offset)
                }
            })
            .is_err();

        if alloc_ok {
            Err(AllocError {})
        } else {
            self.wasted.fetch_add(wasted, Relaxed);
            Ok(unsafe { NonNull::new_unchecked(self.bytes.get().byte_add(alloc_offset)) })
        }
    }

    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {
        // panic!("Can't deallocate from Bump allocator (ptr: {:p}, layout: {:?})", ptr, layout)
    }
}

#[cfg(test)]
mod tests {
    use crate::mem::PAGE_SIZE_4K;

    use super::*;

    #[test]
    fn bump_new() {
        let bump = Bump::<PAGE_SIZE_4K, PAGE_SIZE_4K>::new(0);
        let result = unsafe { bump.allocate(Layout::from_size_align_unchecked(4096, 4096)) };
        assert!(result.is_ok());
        assert_eq!(bump.result_offset(result), Some(0));
        assert_eq!(bump.wasted.load(Relaxed), 0);
        assert_eq!(bump.next_offset.load(Relaxed), 4096);

        // Next should fail - out of space
        let result = unsafe { bump.allocate(Layout::from_size_align_unchecked(1, 1)) };
        assert!(result.is_err());
    }

    #[test]
    fn bump_alignment() {
        let bump = Bump::<{ 3 * PAGE_SIZE_4K }, PAGE_SIZE_4K>::new(0);

        // Small allocation
        let mut expected_waste = 0;
        let result = unsafe { bump.allocate(Layout::from_size_align_unchecked(16, 1)) };
        assert!(result.is_ok());
        assert_eq!(bump.result_offset(result), Some(0));
        assert_eq!(bump.wasted.load(Relaxed), expected_waste);
        assert_eq!(bump.next_offset.load(Relaxed), 16);

        // Align next allocation to 4096, wasting space
        expected_waste += 4096 - 16;
        let result = unsafe { bump.allocate(Layout::from_size_align_unchecked(16, 4096)) };
        assert!(result.is_ok());
        assert_eq!(bump.result_offset(result), Some(4096));
        assert_eq!(bump.wasted.load(Relaxed), expected_waste);
        assert_eq!(bump.next_offset.load(Relaxed), 4096 + 16);

        // Align next allocation to 4096, wasting space
        expected_waste += 4096 - 16;
        let result = unsafe { bump.allocate(Layout::from_size_align_unchecked(4096, 4096)) };
        assert!(result.is_ok());
        assert_eq!(bump.result_offset(result), Some(2 * 4096));
        assert_eq!(bump.wasted.load(Relaxed), expected_waste);
        assert_eq!(bump.next_offset.load(Relaxed), 3 * 4096);
    }

    #[test]
    fn align_too_high() {
        let bump = Bump::<PAGE_SIZE_4K, PAGE_SIZE_4K>::new(0);
        let result = unsafe { bump.allocate(Layout::from_size_align_unchecked(4096, 8192)) };
        assert!(result.is_err());
    }
}
