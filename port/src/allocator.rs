// Copyright 2021  The Hypatia Authors
// All rights reserved
//
// Use of this source code is governed by an MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT.

#![allow(clippy::too_long_first_doc_paragraph)]

use alloc::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{mem, ptr};

/// The allocator works in terms of an owned region of memory
/// that is represented by a Block, which describes the region
/// in terms of a non-nil pointer and a length.  A Block is an
/// analogue of a mutable slice.
///
/// At some point, it may make sense to replace this with a
/// slice pointer, but too many of the interfaces there are not
/// (yet) stable.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    ptr: NonNull<u8>,
    len: usize,
}

impl Block {
    /// Creates a new block from raw parts.  This is analogous
    /// to `core::slice::from_raw_parts`.
    ///
    /// # Safety
    /// The caller must ensure that the pointer and length given
    /// are appropriate for the construction of a new block.
    pub const unsafe fn new_from_raw_parts(ptr: *mut u8, len: usize) -> Block {
        let ptr = unsafe { NonNull::new_unchecked(ptr) };
        Block { ptr, len }
    }

    /// Splits a block into two sub-blocks.
    pub fn split_at_mut(self, offset: usize) -> Option<(Block, Block)> {
        let len = self.len();
        if offset > len {
            return None;
        }
        let ptr = self.as_ptr();
        let a = unsafe { Block::new_from_raw_parts(ptr, offset) };
        let b = unsafe { Block::new_from_raw_parts(ptr.wrapping_add(offset), len - offset) };
        Some((a, b))
    }

    /// Returns a raw mutable pointer to the beginning of the
    /// owned region.
    pub fn as_ptr(self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Returns the length of the region.
    fn len(self) -> usize {
        self.len
    }
}

/// A Bump Allocator takes ownership a region of memory, called
/// an "arena", represented by a Block, and maintains a cursor
/// into that region.  The cursor denotes the point between
/// allocated and unallocated memory in the arena.
pub struct BumpAlloc {
    arena: Block,
    cursor: AtomicUsize,
}

impl BumpAlloc {
    /// Creates a new bump allocator over the given Block.
    /// Takes ownership of the provided region.
    pub const fn new(arena: Block) -> BumpAlloc {
        BumpAlloc { arena, cursor: AtomicUsize::new(0) }
    }

    /// Allocates the requested number of bytes with the given
    /// alignment.  Returns `None` if the allocation cannot be
    /// satisfied, otherwise returns `Some` of a pair of blocks:
    /// the first contains the prefix before the (aligned) block
    /// and the second is the requested block itself.
    pub fn try_alloc(&self, align: usize, size: usize) -> Option<(Block, Block)> {
        let base = self.arena.as_ptr();
        let mut first = ptr::null_mut();
        let mut adjust = 0;
        self.cursor
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                first = base.wrapping_add(current);
                adjust = first.align_offset(align);
                let offset = current.checked_add(adjust).expect("alignment overflow");
                let next = offset.checked_add(size).expect("size overflow");
                (next <= self.arena.len()).then_some(next)
            })
            .ok()?;
        let prefix = unsafe { Block::new_from_raw_parts(first, adjust) };
        let ptr = first.wrapping_add(adjust);
        let block = unsafe { Block::new_from_raw_parts(ptr, size) };
        Some((prefix, block))
    }
}

/// BumpAlloc<T> implements the allocator interface, and is
/// suitable for e.g. page allocators and so forth.  Dealloc is
/// unimplemented and will panic.
unsafe impl Allocator for BumpAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let (_, block) = self.try_alloc(layout.size(), layout.align()).ok_or(AllocError)?;
        Ok(NonNull::slice_from_raw_parts(block.ptr, block.len()))
    }

    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {
        unimplemented!();
    }
}

// # QuickFit allocator for small objects.
//
// This is an implementation of the QuickFit[Wei88] allocator
// for small objects, suitable for managing small heaps in
// memory constrained environments, such as boot loaders and
// standalone debuggers.
//
// [Wei88] Charles B. Weinstock and William A. Wulf. 1988.
// Quick Fit: An Efficient Algorithm for Heap Storage
// Allocation.  ACM SIGPLAN Notices 23, 10 (Oct. 1988),
// 141-148.  https://doi.org/10.1145/51607.51619

const ALLOC_UNIT_SHIFT: usize = 6;
const ALLOC_UNIT_SIZE: usize = 1 << ALLOC_UNIT_SHIFT;
const MIN_ALLOC_SIZE: usize = ALLOC_UNIT_SIZE;
const MAX_QUICK_SHIFT: usize = 14;
const MAX_QUICK_SIZE: usize = 1 << MAX_QUICK_SHIFT;

const NUM_QLISTS: usize = 14 - ALLOC_UNIT_SHIFT + 1;
const NUM_HASH_BUCKETS: usize = 31; // Prime.

/// A linked block header containing size, alignment, and
/// address information for the block.  This is used both for
/// linking unallocated blocks into one of the free lists and
/// for keeping track of blocks allocated from the `misc` list.
///
/// For irregularly sized allocations, the header keeps track of
/// the block's layout data, its virtual address, and a link
/// pointer.  Such a header is either not in any list, if newly
/// allocated and not yet freed, or always in exactly one of two
/// lists: the free list, or a hash chain of allocated blocks.
/// We do this because we need some way to preserve the
/// allocation size after the initial allocation from the tail,
/// and because misc blocks can be reused in a first-fit manner,
/// we cannot rely on a `Layout` to recover the size of the
/// block, so we must store it somewhere.  By allocating a tag
/// outside of the buffer, which we look up in a hash table as
/// needed, we can maintain this information without adding
/// additional complexity to allocation.
///
/// For blocks on one of the quick lists, the size, address and
/// alignment fields are redundant, but convenient.
///
/// We use the link pointer to point to the next entry in the
/// list in all cases.
#[derive(Debug)]
#[repr(C, align(64))]
struct Header {
    next: Option<NonNull<Header>>,
    addr: NonNull<u8>,
    size: usize,
    align: usize,
}

impl Header {
    /// Returns a new header for a block of the given size and
    /// alignment at the given address.
    fn new(addr: NonNull<u8>, size: usize, align: usize, next: Option<NonNull<Header>>) -> Header {
        Header { next, addr, size, align }
    }
}

/// The QuickFit allocator itself.  The allocator takes
/// ownership of a bump allocator for the tail, and contains a
/// set of lists for the quick blocks, as well as a misc list
/// for unusually sized regions, and a hash table of headers
/// describing current misc allocations.  As mentioned above,
/// these last data are kept outside of the allocations to keep
/// allocation simple.
#[repr(C)]
pub struct QuickFit {
    tail: BumpAlloc,
    qlists: [Option<NonNull<Header>>; NUM_QLISTS],
    misc: Option<NonNull<Header>>,
    allocated_misc: [Option<NonNull<Header>>; NUM_HASH_BUCKETS],
}

impl QuickFit {
    /// Constructs a QuickFit from the given `tail`.
    pub const fn new(tail: BumpAlloc) -> QuickFit {
        let qlists = [None; NUM_QLISTS];
        let misc = None;
        let allocated_misc = [None; NUM_HASH_BUCKETS];
        QuickFit { tail, qlists, misc, allocated_misc }
    }

    /// Allocates a block of memory of the requested size and
    /// alignment.  Returns a pointer to such a block, or nil if
    /// the block cannot be allocated.
    pub fn malloc(&mut self, layout: Layout) -> *mut u8 {
        let (size, align) = Self::adjust(layout);
        let p = self.alloc_quick(size, align);
        p.or_else(|| self.alloc_tail(size, align)).map(|p| p.as_ptr()).unwrap_or(ptr::null_mut())
    }

    /// Adjusts the given layout so that blocks allocated from
    /// one of the quick lists are appropriately sized and
    /// aligned.  Otherwise, returns the original size and
    /// alignment.
    fn adjust(layout: Layout) -> (usize, usize) {
        let size = layout.size();
        let align = layout.align();
        if size > MAX_QUICK_SIZE {
            return (size, align);
        }
        let size = usize::max(MIN_ALLOC_SIZE, size.next_power_of_two());
        let align = usize::max(layout.align(), size);
        (size, align)
    }

    /// Attempts to allocate from an existing list: for requests
    /// that can be satisfied from one of the quick lists, try
    /// and do so; otherwise, attempt an allocation from the
    /// misc list.
    fn alloc_quick(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        if size <= MAX_QUICK_SIZE && align == size {
            let k: usize = size.ilog2() as usize - ALLOC_UNIT_SHIFT;
            let (node, list) = Self::head(self.qlists[k].take());
            self.qlists[k] = list;
            node.map(|header| unsafe { header.as_ref() }.addr)
        } else {
            self.alloc_misc(size, align)
        }
    }

    /// Allocates a block from the misc list.  This is a simple
    /// first-fit allocator.
    fn alloc_misc(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let (node, list) =
            Self::unlink(self.misc.take(), |node| size <= node.size && align <= node.align);
        self.misc = list;
        node.map(|mut header| {
            let header = unsafe { header.as_mut() };
            let k = Self::hash(header.addr.as_ptr());
            header.next = self.allocated_misc[k].take();
            self.allocated_misc[k] = NonNull::new(header);
            header.addr
        })
    }

    /// Allocates an aligned block of size `size` from `tail`.
    /// If `tail` is not already aligned to the given alignment,
    /// then we try to free blocks larger than or equal in size
    /// to the minimum allocation unit into the quick lists
    /// until it is.
    fn alloc_tail(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let (prefix, block) = { self.tail.try_alloc(size, align)? };
        self.free_prefix(prefix);
        Some(block.ptr)
    }

    /// Frees a prefix that came from a tail allocation.  This
    /// attempts to store blocks into the quick lists.
    fn free_prefix(&mut self, prefix: Block) {
        let mut prefix = Self::align_prefix(prefix);
        while let Some(rest) = self.try_free_prefix(prefix) {
            prefix = rest;
        }
    }

    /// Aligns the prefix to the minimum allocation size.
    fn align_prefix(prefix: Block) -> Block {
        let ptr = prefix.as_ptr();
        let len = prefix.len();
        let offset = ptr.align_offset(MIN_ALLOC_SIZE);
        assert!(offset <= len);
        unsafe { Block::new_from_raw_parts(ptr.wrapping_add(offset), len - offset) }
    }

    /// Tries to free the largest section of the prefix that it
    /// can, returning the remainder if it did so.  Otherwise,
    /// returns None.
    fn try_free_prefix(&mut self, prefix: Block) -> Option<Block> {
        let ptr: *mut u8 = prefix.as_ptr();
        for k in (0..NUM_QLISTS).rev() {
            let size = 1 << (k + ALLOC_UNIT_SHIFT);
            if prefix.len() >= size && ptr.align_offset(size) == 0 {
                let (_, rest) = prefix.split_at_mut(size)?;
                self.free(ptr, Layout::from_size_align(size, size).unwrap());
                return (rest.len() >= MIN_ALLOC_SIZE).then_some(rest);
            }
        }
        None
    }

    /// Attempts to reallocate the given block to a new size.
    ///
    /// This has a small optimization for the most common case,
    /// where a block is being realloc'd to grow as data is
    /// accumulated: it's subtle, but if the original block was
    /// allocated from one of the quick lists, and the new size
    /// can be accommodated by the existing allocation, simply
    /// return the existing block pointer.  Otherwise, allocate
    /// a new block, copy, and free the old block.
    ///
    /// Note that the case of a reduction in size might result
    /// in a new allocation.  This is because we rely on the
    /// accuracy of the `Layout` to find the correct quicklist
    /// to store the block onto on free.  If we reduced below
    /// the size of the current block, we would lose the layout
    /// information and potentially leak memory.  But this is
    /// very uncommon.
    ///
    /// We make no effort to optimize the case of a `realloc` in
    /// a `misc` block, as a) it is relatively uncommon to do so
    /// and b) there may not be a buffer tag for such a block
    /// yet (one isn't allocated until the block is freed), and
    /// the implementation would need to be more complex as a
    /// result.
    ///
    /// # Safety
    /// Must be called with a valid block pointer, layout, and
    /// size.
    pub unsafe fn realloc(&mut self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if block.is_null() {
            return self.malloc(layout);
        }
        let new_layout = Layout::from_size_align(new_size, layout.align()).expect("layout");
        let (size, align) = Self::adjust(new_layout);
        if size == layout.size() && align == layout.align() {
            return block;
        }
        let np = self.malloc(new_layout);
        if !np.is_null() {
            unsafe {
                ptr::copy(block, np, usize::min(layout.size(), new_size));
            }
            self.free(block, layout)
        }
        np
    }

    /// Frees a block of memory characterized by the `layout`
    /// argument.  If the block can be freed to one of the
    /// quick lists, it is; otherwise, it is treated as a misc
    /// block and freed there.
    pub fn free(&mut self, block: *mut u8, layout: Layout) {
        let Some(block) = NonNull::new(block) else {
            return;
        };
        let (size, align) = Self::adjust(layout);
        if size <= MAX_QUICK_SIZE && align == size {
            let k: usize = size.ilog2() as usize - ALLOC_UNIT_SHIFT;
            let header = Header::new(block, size, align, self.qlists[k].take());
            assert_eq!(block.align_offset(mem::align_of::<Header>()), 0);
            let p = block.cast::<Header>();
            unsafe {
                ptr::write(p.as_ptr(), header);
            }
            self.qlists[k] = Some(p);
        } else {
            self.free_misc(block, size, align);
        }
    }

    /// Frees a block to the misc list.  This looks up the given
    /// address in the hash of allocated misc blocks to find its
    /// header.
    ///
    /// If the block header is not found in the hash table, we
    /// assume that the block was allocated from the tail and
    /// this is the first time it's been freed, so we allocate a
    /// header for it and link that into the misc list.
    ///
    /// If we cannot allocate a header in the usual way, we take
    /// it from the block to be freed, which is guaranteed to be
    /// large enough to hold a header, since anything smaller
    /// would have been allocated from one of the quick lists,
    /// and thus freed through that path.
    fn free_misc(&mut self, mut block: NonNull<u8>, mut size: usize, mut align: usize) {
        let mut header = self
            .unlink_allocated_misc(block)
            .or_else(|| {
                let hblock = self.malloc(Layout::new::<Header>()).cast::<Header>();
                let hblock = hblock
                    .is_null()
                    .then(|| {
                        let offset = block.align_offset(MIN_ALLOC_SIZE);
                        let hblock = block.as_ptr().wrapping_add(offset);
                        let next = hblock.wrapping_add(MIN_ALLOC_SIZE);
                        block = unsafe { NonNull::new_unchecked(next) };
                        size -= offset + MIN_ALLOC_SIZE;
                        align = MIN_ALLOC_SIZE;
                        hblock.cast()
                    })
                    .expect("allocated header block");
                let header = Header::new(block, size, align, None);
                unsafe {
                    ptr::write(hblock, header);
                }
                NonNull::new(hblock)
            })
            .expect("header");
        let header = unsafe { header.as_mut() };
        header.next = self.misc.take();
        self.misc = NonNull::new(header);
    }

    /// Unlinks the header for the given address from the hash
    /// table for allocated misc blocks and returns it, if such
    /// a header exists.  If the block associated with the
    /// address has not been freed yet, it's possible that no
    /// header for it exists yet, in which case we return None.
    fn unlink_allocated_misc(&mut self, block: NonNull<u8>) -> Option<NonNull<Header>> {
        let k = Self::hash(block.as_ptr());
        let list = self.allocated_misc[k].take();
        let (node, list) = Self::unlink(list, |node| node.addr == block);
        self.allocated_misc[k] = list;
        node
    }

    /// Unlinks the first node matching the given predicate from
    /// the given list, if it exists, returning the node, or
    /// None, and the list head.  The list head will be None if
    /// the list is empty.
    fn unlink<F>(
        mut list: Option<NonNull<Header>>,
        predicate: F,
    ) -> (Option<NonNull<Header>>, Option<NonNull<Header>>)
    where
        F: Fn(&Header) -> bool,
    {
        let mut prev: Option<NonNull<Header>> = None;
        while let Some(mut node) = list {
            let node = unsafe { node.as_mut() };
            if predicate(node) {
                let next = node.next.take();
                if let Some(mut prev) = prev {
                    let prev = unsafe { prev.as_mut() };
                    prev.next = next;
                } else {
                    list = next;
                }
                return (NonNull::new(node), list);
            }
            prev = NonNull::new(node);
            list = node.next;
        }
        (None, list)
    }

    /// Splits the list into it's first element and tail and
    /// returns both.
    fn head(list: Option<NonNull<Header>>) -> (Option<NonNull<Header>>, Option<NonNull<Header>>) {
        Self::unlink(list, |_| true)
    }

    /// Hashes a pointer value.  This is the bit mixing algorithm
    /// from Murmur3.
    fn hash(ptr: *mut u8) -> usize {
        let mut k = ptr.addr();
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51afd7ed558ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ceb9fe1a85ec53);
        (k >> 33) % NUM_HASH_BUCKETS
    }
}

#[cfg(not(test))]
pub mod global {
    use super::QuickFit;
    use alloc::alloc::{GlobalAlloc, Layout};
    use core::ptr;
    use core::sync::atomic::{AtomicPtr, Ordering};

    const GLOBAL_HEAP_SIZE: usize = 4 * 1024 * 1024;

    /// A GlobalHeap is an aligned wrapper around an owned
    /// buffer.
    #[repr(C, align(4096))]
    pub struct GlobalHeap([u8; GLOBAL_HEAP_SIZE]);
    impl GlobalHeap {
        pub const fn new() -> GlobalHeap {
            Self([0u8; GLOBAL_HEAP_SIZE])
        }
    }

    impl Default for GlobalHeap {
        fn default() -> Self {
            Self::new()
        }
    }

    /// GlobalQuickAlloc is a wrapper around a QuickFit over a
    /// GlobalHeap that uses interior mutability to implement
    /// the GlobalAlloc trait.
    pub struct GlobalQuickAlloc(pub AtomicPtr<QuickFit>);
    impl GlobalQuickAlloc {
        fn with_allocator<F, R>(&self, thunk: F) -> R
        where
            F: FnOnce(&mut QuickFit) -> R,
        {
            let a = self.0.swap(ptr::null_mut(), Ordering::Relaxed);
            assert!(!a.is_null(), "global allocator is nil");
            let r = thunk(unsafe { &mut *a });
            self.0.swap(a, Ordering::Relaxed);
            r
        }
    }

    unsafe impl GlobalAlloc for GlobalQuickAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            self.with_allocator(|quick| quick.malloc(layout))
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            self.with_allocator(|quick| quick.free(ptr, layout));
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            self.with_allocator(|quick| unsafe { quick.realloc(ptr, layout, new_size) })
        }
    }
}
