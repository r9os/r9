use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Bump allocator to be used for earliest allocations in r9.  These allocations
/// can never be freed - attempting to do so will panic.
/// This has been originally based on the example here:
/// https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html
#[repr(C, align(4096))]
pub struct Bump<const SIZE_BYTES: usize, const MAX_SUPPORTED_ALIGN: usize> {
    bytes: UnsafeCell<[u8; SIZE_BYTES]>,
    remaining: AtomicUsize,
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
            remaining: AtomicUsize::new(SIZE_BYTES),
        }
    }
}

unsafe impl<const SIZE_BYTES: usize, const MAX_SUPPORTED_ALIGN: usize> GlobalAlloc
    for Bump<SIZE_BYTES, MAX_SUPPORTED_ALIGN>
{
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        if align > MAX_SUPPORTED_ALIGN {
            return null_mut();
        }

        let mut allocated = 0;
        if self
            .remaining
            .fetch_update(Relaxed, Relaxed, |mut remaining| {
                if size > remaining {
                    return None;
                }

                // `Layout` contract forbids making a `Layout` with align=0, or
                // align not power of 2.  So we can safely use a mask to ensure
                // alignment without worrying about UB.
                let align_mask_to_round_down = !(align - 1);

                remaining -= size;
                remaining &= align_mask_to_round_down;
                allocated = remaining;
                Some(remaining)
            })
            .is_err()
        {
            null_mut()
        } else {
            unsafe { self.bytes.get().cast::<u8>().add(allocated) }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        panic!("Can't dealloc from Bump allocator (ptr: {:p}, layout: {:?})", ptr, layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_new() {
        let bump = Bump::<4096, 4096>::new(0);
        let ptr = unsafe { bump.alloc(Layout::from_size_align_unchecked(4096, 4096)) };
        assert!(!ptr.is_null());
        let ptr = unsafe { bump.alloc(Layout::from_size_align_unchecked(1, 1)) };
        assert!(ptr.is_null());
    }

    #[test]
    fn align_too_high() {
        let bump = Bump::<4096, 4096>::new(0);
        let ptr = unsafe { bump.alloc(Layout::from_size_align_unchecked(4096, 8192)) };
        assert!(ptr.is_null());
    }
}
