use core::{ops::Range, ptr::null_mut, slice};

use crate::mem::VirtRange;

#[cfg(not(test))]
use crate::println;

// TODO reserve recursive area in vmem(?)
// TODO Add hashtable for allocated tags - makes it faster when freeing, given only an address.
// TODO Add support for quantum caches once we have slab allocators implemented.
// TODO Add power-of-two freelists for freed allocations.

#[derive(Debug, PartialEq)]
pub enum BoundaryError {
    ZeroSize,
}

#[derive(Debug, PartialEq)]
pub enum AllocError {
    NoSpace,
    AllocationNotFound,
}

#[cfg(test)]
type BoundaryResult<T> = core::result::Result<T, BoundaryError>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boundary {
    start: usize,
    size: usize,
}

impl Boundary {
    #[cfg(test)]
    fn new(start: usize, size: usize) -> BoundaryResult<Self> {
        if size == 0 {
            Err(BoundaryError::ZeroSize)
        } else {
            Ok(Self { start, size })
        }
    }

    fn new_unchecked(start: usize, size: usize) -> Self {
        Self { start, size }
    }

    #[allow(dead_code)]
    fn overlaps(&self, other: &Boundary) -> bool {
        let boundary_end = self.start + self.size;
        let tag_end = other.start + other.size;
        (self.start <= other.start && boundary_end > other.start)
            || (self.start < tag_end && boundary_end >= tag_end)
            || (self.start <= other.start && boundary_end >= tag_end)
    }

    #[allow(dead_code)]
    fn end(&self) -> usize {
        self.start + self.size
    }
}

impl From<VirtRange> for Boundary {
    fn from(r: VirtRange) -> Self {
        Boundary::new_unchecked(r.start(), r.size())
    }
}

impl From<Range<usize>> for Boundary {
    fn from(r: Range<usize>) -> Self {
        Boundary::new_unchecked(r.start, r.end - r.start)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TagType {
    Allocated,
    Free,
    Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tag {
    tag_type: TagType,
    boundary: Boundary,
}

impl Tag {
    fn new(tag_type: TagType, boundary: Boundary) -> Self {
        Self { tag_type, boundary }
    }

    #[cfg(test)]
    fn new_allocated(boundary: Boundary) -> Self {
        Tag::new(TagType::Allocated, boundary)
    }

    fn new_free(boundary: Boundary) -> Self {
        Tag::new(TagType::Free, boundary)
    }

    fn new_span(boundary: Boundary) -> Self {
        Tag::new(TagType::Span, boundary)
    }
}

// impl fmt::Debug for Tag {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         write!(
//             f,
//             "Tag({:?} {}..{} (size: {}))",
//             self.tag_type,
//             self.boundary.start,
//             self.boundary.start + self.boundary.size,
//             self.boundary.size
//         )?;
//         Ok(())
//     }
// }

#[derive(Debug)]
struct TagItem {
    tag: Tag,
    next: *mut TagItem,
    prev: *mut TagItem,
}

impl TagItem {
    #[cfg(test)]
    fn new_allocated(boundary: Boundary) -> Self {
        Self { tag: Tag::new_allocated(boundary), next: null_mut(), prev: null_mut() }
    }
}

/// Pool of boundary tags.  Vmem uses external boundary tags.  We allocate a page
/// of tags at a time, making them available via this pool.  This allows us to
/// set up the pool initially with a static page, before we have any kind of
/// allocator.  The pool can later be populated dynamically.
struct TagPool {
    tags: *mut TagItem,
}

impl TagPool {
    fn new() -> Self {
        Self { tags: null_mut() }
    }

    fn add(&mut self, tag: &mut TagItem) {
        if self.tags.is_null() {
            self.tags = tag;
        } else {
            tag.next = self.tags;
            unsafe { (*tag.next).prev = tag };
            self.tags = tag;
        }
    }

    fn take(&mut self, tag: Tag) -> *mut TagItem {
        if let Some(tag_item) = unsafe { self.tags.as_mut() } {
            self.tags = tag_item.next;
            if let Some(next_tag) = unsafe { self.tags.as_mut() } {
                next_tag.prev = null_mut();
            }
            tag_item.next = null_mut();
            tag_item.prev = null_mut();
            tag_item.tag = tag;
            tag_item as *mut TagItem
        } else {
            null_mut()
        }
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        let mut n = 0;
        let mut free_tag = self.tags;
        while let Some(tag) = unsafe { free_tag.as_ref() } {
            n += 1;
            free_tag = tag.next;
        }
        n
    }
}

/// Ordered list of tags (by Tag::start)
/// This is a simple linked list that assumes no overlaps.
struct TagList {
    tags: *mut TagItem,
}

impl TagList {
    fn new() -> Self {
        Self { tags: null_mut() }
    }

    fn push(&mut self, new_tag: &mut TagItem) {
        if self.tags.is_null() {
            self.tags = new_tag;
        } else {
            let mut curr_tag_item = self.tags;
            while let Some(item) = unsafe { curr_tag_item.as_mut() } {
                if item.tag.boundary.start > new_tag.tag.boundary.start {
                    // Insert before tag
                    if let Some(prev_tag) = unsafe { item.prev.as_mut() } {
                        prev_tag.next = new_tag;
                    } else {
                        // Inserting as first tag
                        self.tags = new_tag;
                    }
                    new_tag.next = item;
                    item.prev = new_tag;
                    return;
                }
                if item.next.is_null() {
                    // Inserting as last tag
                    new_tag.prev = item;
                    item.next = new_tag;
                    return;
                }
                curr_tag_item = item.next;
            }
        }
    }

    /// Remove tag_item from the list.  Placing tag_item onto the free list is
    /// the callers responsibility.
    fn unlink(tag_item: &mut TagItem) {
        if let Some(prev) = unsafe { tag_item.prev.as_mut() } {
            prev.next = tag_item.next;
        }
        if let Some(next) = unsafe { tag_item.next.as_mut() } {
            next.prev = tag_item.prev;
        }
        tag_item.next = null_mut();
        tag_item.prev = null_mut();
    }

    fn len(&self) -> usize {
        let mut n = 0;
        let mut curr_tag = self.tags;
        while let Some(tag) = unsafe { curr_tag.as_ref() } {
            n += 1;
            curr_tag = tag.next;
        }
        n
    }

    fn tags_iter(&self) -> impl Iterator<Item = Tag> + '_ {
        let mut curr_tag_item = self.tags;
        core::iter::from_fn(move || {
            if let Some(item) = unsafe { curr_tag_item.as_ref() } {
                curr_tag_item = item.next;
                Some(item.tag)
            } else {
                None
            }
        })
    }

    // fn add_tag(&mut self, boundary: Boundary, free_tags: &mut TagStack) -> BoundaryResult<()> {
    //     // Code to pop a tag
    //     // let tag = unsafe {
    //     //     arena.free_tags.pop().as_mut().expect("Arena::new_with_tags no free tags")
    //     // };

    //     if boundary.size == 0 {
    //         return Err(BoundaryError::ZeroSize);
    //     }

    //     let bstart = boundary.start;
    //     let bend = boundary.start + boundary.size;

    //     let mut curr_tag = self.tags;
    //     while let Some(tag) = unsafe { curr_tag.as_ref() } {
    //         let tag_start = tag.boundary.start;
    //         let tag_end = tag_start + tag.boundary.size;
    //         if (bstart <= tag_start && bend > tag_start)
    //             || (bstart < tag_end && bend >= tag_end)
    //             || (bstart <= tag_start && bend >= tag_end)
    //         {}
    //         curr_tag = tag.next;
    //     }

    //     Ok(())
    // }
}

// TODO this needs to be Sync, so actually make it sync
pub struct Arena {
    name: &'static str,
    quantum: usize,

    tag_pool: TagPool, // Pool of available tags
    segment_list: TagList, // List of all segments in address order

                       //parent: Option<&Arena>, // Parent arena to import from
}

unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}

pub trait Allocator {
    fn alloc(&mut self, size: usize) -> *mut u8;
    fn free(&mut self, addr: *mut u8);
}

impl Arena {
    pub fn new(
        name: &'static str,
        initial_span: Option<Boundary>,
        quantum: usize,
        _parent: Option<Arena>,
    ) -> Self {
        println!("Arena::new name:{} initial_span:{:?} quantum:{:x}", name, initial_span, quantum);

        let mut arena =
            Self { name, quantum, segment_list: TagList::new(), tag_pool: TagPool::new() };

        if let Some(span) = initial_span {
            arena.add_initial_span(span);
        }

        arena
    }

    /// Only to be used for creation of initial heap
    /// Create a new arena, assuming there is no dynamic allocation available,
    /// and all free tags come from the free_tags provided.
    pub fn new_with_static_range(
        name: &'static str,
        initial_span: Option<Boundary>,
        quantum: usize,
        static_range: VirtRange,
    ) -> Self {
        let tags_addr = unsafe { &mut *(static_range.start() as *mut TagItem) };
        let tags = unsafe {
            slice::from_raw_parts_mut(tags_addr, static_range.size() / size_of::<TagItem>())
        };

        Self::new_with_tags(name, initial_span, quantum, tags)
    }

    /// Only to be used for creation of initial heap
    /// Create a new arena, assuming there is no dynamic allocation available,
    /// and all free tags come from the free_tags provided.
    fn new_with_tags(
        name: &'static str,
        initial_span: Option<Boundary>,
        quantum: usize,
        tags: &mut [TagItem],
    ) -> Self {
        println!(
            "Arena::new_with_tags name:{} initial_span:{:?} quantum:{:x}",
            name, initial_span, quantum
        );

        let mut arena =
            Self { name, quantum, segment_list: TagList::new(), tag_pool: TagPool::new() };
        arena.add_tags_to_pool(tags);

        if let Some(span) = initial_span {
            arena.add_initial_span(span);
        }

        arena
    }

    fn add_initial_span(&mut self, span: Boundary) {
        assert_eq!(span.start % self.quantum, 0);
        assert_eq!(span.size % self.quantum, 0);
        assert!(span.start.checked_add(span.size).is_some());
        self.add_free_span(span);
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    fn add_free_span(&mut self, boundary: Boundary) {
        self.segment_list.push(unsafe {
            self.tag_pool.take(Tag::new_span(boundary)).as_mut().expect("no free tags")
        });
        self.segment_list.push(unsafe {
            self.tag_pool.take(Tag::new_free(boundary)).as_mut().expect("no free tags")
        });
    }

    fn add_tags_to_pool(&mut self, tags: &mut [TagItem]) {
        for tag in tags {
            tag.next = null_mut();
            tag.prev = null_mut();
            self.tag_pool.add(tag);
        }
    }

    /// Allocate a segment, returned as a boundary
    fn alloc_segment(&mut self, size: usize) -> Result<Boundary, AllocError> {
        println!("alloc_segment size: {}", size);

        // Round size up to a multiple of quantum
        let size = {
            let rem = size % self.quantum;
            if rem == 0 {
                size
            } else {
                size + (self.quantum - rem)
            }
        };

        // Find the first free tag that's large enough
        let mut curr_item = self.segment_list.tags;
        while let Some(item) = unsafe { curr_item.as_mut() } {
            if item.tag.tag_type == TagType::Free && item.tag.boundary.size >= size {
                // Mark this tag as allocated, and if there's any left over space,
                // create and insert a new tag
                item.tag.tag_type = TagType::Allocated;
                if item.tag.boundary.size > size {
                    // Work out the size of the new free item, and change the size
                    // of the current, now allocated, item
                    let remainder = item.tag.boundary.size - size;
                    item.tag.boundary.size = size;

                    let new_tag = Tag::new_free(Boundary::new_unchecked(
                        item.tag.boundary.start + size,
                        remainder,
                    ));
                    let new_item =
                        unsafe { self.tag_pool.take(new_tag).as_mut().expect("no free tags") };

                    // Insert new_item after item
                    new_item.next = item.next;
                    new_item.prev = item;
                    item.next = new_item;
                    if !new_item.next.is_null() {
                        unsafe { (*new_item.next).prev = new_item };
                    }
                }
                return Ok(item.tag.boundary);
            }
            curr_item = item.next;
        }
        Err(AllocError::NoSpace)
    }

    // Free addr.  We don't need to know size because we don't merge allocations.
    // (We only merge freed segments)
    // TODO Error on precondition fail
    fn free_segment(&mut self, addr: usize) -> Result<(), AllocError> {
        // Need to manually scan the used tags
        let mut curr_item = self.segment_list.tags;
        while let Some(item) = unsafe { curr_item.as_mut() } {
            if item.tag.boundary.start == addr && item.tag.tag_type == TagType::Allocated {
                break;
            }
            curr_item = item.next;
        }

        if curr_item.is_null() {
            return Err(AllocError::AllocationNotFound);
        }

        let curr_tag: &mut TagItem = unsafe { curr_item.as_mut() }.unwrap();

        // Found tag to free
        let prev_type = unsafe { curr_tag.prev.as_ref() }.map(|t| t.tag.tag_type);
        let next_type = unsafe { curr_tag.next.as_ref() }.map(|t| t.tag.tag_type);

        match (prev_type, next_type) {
            (Some(TagType::Allocated), Some(TagType::Allocated))
            | (Some(TagType::Span), Some(TagType::Span))
            | (Some(TagType::Span), Some(TagType::Allocated))
            | (Some(TagType::Allocated), Some(TagType::Span))
            | (Some(TagType::Span), None)
            | (Some(TagType::Allocated), None) => {
                // No frees on either side
                // -> Change curr_tag to free
                curr_tag.tag.tag_type = TagType::Free;
            }
            (Some(TagType::Span), Some(TagType::Free))
            | (Some(TagType::Allocated), Some(TagType::Free)) => {
                // Prev non-free, next free
                // Change next tag start to merge with curr_tag, release curr_tag
                let next = unsafe { curr_tag.next.as_mut() }.unwrap();
                next.tag.boundary.start = curr_tag.tag.boundary.start;
                next.tag.boundary.size += curr_tag.tag.boundary.size;
                TagList::unlink(curr_tag);
                self.tag_pool.add(curr_tag);
            }
            (Some(TagType::Free), None)
            | (Some(TagType::Free), Some(TagType::Span))
            | (Some(TagType::Free), Some(TagType::Allocated)) => {
                // Prev free, next non-free
                // Change prev tag size to merge with curr_tag, release curr_tag
                let prev = unsafe { curr_tag.prev.as_mut() }.unwrap();
                prev.tag.boundary.size += curr_tag.tag.boundary.size;
                TagList::unlink(curr_tag);
                self.tag_pool.add(curr_tag);
            }
            (Some(TagType::Free), Some(TagType::Free)) => {
                // Prev and next both free
                // Change prev size to merge with both curr_tag and next, release curr_tag
                let prev = unsafe { curr_tag.prev.as_mut() }.unwrap();
                let next = unsafe { curr_tag.next.as_mut() }.unwrap();
                prev.tag.boundary.size += curr_tag.tag.boundary.size + next.tag.boundary.size;
                TagList::unlink(curr_tag);
                TagList::unlink(next);
                self.tag_pool.add(curr_tag);
                self.tag_pool.add(next);
            }
            (None, None)
            | (None, Some(TagType::Span))
            | (None, Some(TagType::Allocated))
            | (None, Some(TagType::Free)) => {
                self.assert_tags_are_consistent();
                panic!("Unexpected tags when freeing");
            }
        }

        Ok(())
    }

    fn tags_iter(&self) -> impl Iterator<Item = Tag> + '_ {
        self.segment_list.tags_iter()
    }

    /// Checks that all invariants are correct.
    fn assert_tags_are_consistent(&self) {
        // There must be at least 2 tags
        debug_assert!(self.segment_list.len() >= 2);

        // Tags must be in order, without gaps
        let mut last_tag: Option<Tag> = None;
        let mut last_span: Option<Tag> = None;
        let mut last_span_total = 0;
        for (i, tag) in self.tags_iter().enumerate() {
            debug_assert!(tag.boundary.size > 0);

            if i == 0 {
                debug_assert_eq!(tag.tag_type, TagType::Span);
                debug_assert!(last_tag.is_none());
                debug_assert!(last_span.is_none());
                debug_assert_eq!(last_span_total, 0);
            } else {
                debug_assert!(last_tag.is_some());
                debug_assert!(last_span.is_some());

                // Tags should be ordered
                let last_tag = last_tag.unwrap();
                let out_of_order = (last_tag.tag_type == TagType::Span
                    && tag.boundary.start >= last_tag.boundary.start)
                    || (last_tag.tag_type != TagType::Span
                        && tag.boundary.start > last_tag.boundary.start);
                debug_assert!(
                    out_of_order,
                    "Tags out of order: tag{}: {:?}, tag{}: {:?}",
                    i - 1,
                    last_tag,
                    i,
                    tag,
                );
            }

            match tag.tag_type {
                TagType::Span => {
                    // Spans must not overlap
                    if last_span.is_some() {
                        debug_assert_eq!(last_span_total, last_span.unwrap().boundary.size);
                    }
                    last_span = Some(tag);
                }
                TagType::Allocated | TagType::Free => {
                    last_span_total += tag.boundary.size;
                    // First tag after span should have same start as span
                    if last_tag.is_some_and(|t| t.tag_type == TagType::Span) {
                        debug_assert_eq!(tag.boundary.start, last_tag.unwrap().boundary.start);
                    }
                }
            }
            last_tag = Some(tag);
        }
    }
}

impl Allocator for Arena {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        let boundary = self.alloc_segment(size);
        if let Ok(boundary) = boundary {
            boundary.start as *mut u8
        } else {
            null_mut()
        }
    }

    fn free(&mut self, addr: *mut u8) {
        let _ = self.free_segment(addr as usize);
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn ensure_sizes() {
        assert_eq!(size_of::<Tag>(), 24);
        assert_eq!(size_of::<TagItem>(), 40);
    }

    #[test]
    fn test_boundary() {
        assert!(Boundary::new(10, 1).is_ok());
        assert_eq!(Boundary::new(10, 0), BoundaryResult::Err(BoundaryError::ZeroSize));

        // Overlap left
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(2, 5).unwrap()));
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(0, 10).unwrap()));
        assert!(Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(0, 11).unwrap()));

        // Overlap right
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(25, 5).unwrap()));
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(20, 10).unwrap()));
        assert!(Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(19, 1).unwrap()));

        // Exact match
        assert!(Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(10, 10).unwrap()));

        // Inside
        assert!(Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(15, 1).unwrap()));
        assert!(Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(10, 1).unwrap()));
        assert!(Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(19, 1).unwrap()));

        // Outside left
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(0, 1).unwrap()));
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(0, 10).unwrap()));

        // Outside right
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(20, 1).unwrap()));
        assert!(!Boundary::new(10, 10).unwrap().overlaps(&Boundary::new(25, 1).unwrap()));
    }

    // Page4K would normally be in the arch crate, but we define something
    // similar here for testing.
    #[repr(C, align(4096))]
    #[derive(Clone, Copy)]
    pub struct Page4K([u8; 4096]);

    #[test]
    fn test_tagstack() {
        let mut page = Page4K([0; 4096]);
        const NUM_TAGS: usize = size_of::<Page4K>() / size_of::<TagItem>();
        let tags = unsafe { &mut *(&mut page as *mut Page4K as *mut [TagItem; NUM_TAGS]) };
        let mut tag_stack = TagPool::new();

        assert_eq!(tag_stack.len(), 0);
        for tag in tags {
            tag_stack.add(tag);
        }
        assert_eq!(tag_stack.len(), NUM_TAGS);
    }

    #[test]
    fn test_taglist() {
        let mut list = TagList::new();
        assert_eq!(list.len(), 0);
        assert_eq!(list.tags_iter().collect::<Vec<Tag>>(), []);

        let mut tag1 = TagItem::new_allocated(Boundary::new(100, 100).unwrap());
        list.push(&mut tag1);
        assert_eq!(list.len(), 1);
        assert_eq!(
            list.tags_iter().collect::<Vec<Tag>>(),
            [Tag::new_allocated(Boundary::new(100, 100).unwrap())]
        );

        // Insert new at end
        let mut tag2 = TagItem::new_allocated(Boundary::new(500, 100).unwrap());
        list.push(&mut tag2);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list.tags_iter().collect::<Vec<Tag>>(),
            [
                Tag::new_allocated(Boundary::new(100, 100).unwrap()),
                Tag::new_allocated(Boundary::new(500, 100).unwrap())
            ]
        );

        // Insert new at start
        let mut tag3 = TagItem::new_allocated(Boundary::new(0, 100).unwrap());
        list.push(&mut tag3);
        assert_eq!(list.len(), 3);
        assert_eq!(
            list.tags_iter().collect::<Vec<Tag>>(),
            [
                Tag::new_allocated(Boundary::new(0, 100).unwrap()),
                Tag::new_allocated(Boundary::new(100, 100).unwrap()),
                Tag::new_allocated(Boundary::new(500, 100).unwrap())
            ]
        );

        // Insert new in middle
        let mut tag4 = TagItem::new_allocated(Boundary::new(200, 100).unwrap());
        list.push(&mut tag4);
        assert_eq!(list.len(), 4);
        assert_eq!(
            list.tags_iter().collect::<Vec<Tag>>(),
            [
                Tag::new_allocated(Boundary::new(0, 100).unwrap()),
                Tag::new_allocated(Boundary::new(100, 100).unwrap()),
                Tag::new_allocated(Boundary::new(200, 100).unwrap()),
                Tag::new_allocated(Boundary::new(500, 100).unwrap())
            ]
        );
    }

    fn create_arena_with_static_tags(
        name: &'static str,
        initial_span: Option<Boundary>,
        quantum: usize,
        _parent_arena: Option<&mut Arena>,
    ) -> Arena {
        let mut page = Page4K([0; 4096]);
        const NUM_TAGS: usize = size_of::<Page4K>() / size_of::<TagItem>();
        let tags = unsafe { &mut *(&mut page as *mut Page4K as *mut [TagItem; NUM_TAGS]) };
        Arena::new_with_tags(name, initial_span, quantum, tags)
    }

    fn assert_tags_eq(arena: &Arena, expected: &[Tag]) {
        arena.assert_tags_are_consistent();
        let actual_tags = arena.tags_iter().collect::<Vec<Tag>>();
        assert_eq!(actual_tags, expected, "arena tag mismatch");
    }

    #[test]
    fn test_arena_create() {
        let arena = create_arena_with_static_tags(
            "test",
            Some(Boundary::new_unchecked(4096, 4096 * 20)),
            4096,
            None,
        );
        assert_eq!(arena.tag_pool.len(), 100);

        assert_tags_eq(
            &arena,
            &[
                Tag::new_span(Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new_free(Boundary::new(4096, 4096 * 20).unwrap()),
            ],
        );
    }

    #[test]
    fn test_arena_alloc() {
        let mut arena = create_arena_with_static_tags(
            "test",
            Some(Boundary::new_unchecked(4096, 4096 * 20)),
            4096,
            None,
        );

        arena.alloc(4096 * 2);

        assert_tags_eq(
            &arena,
            &[
                Tag::new_span(Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new_allocated(Boundary::new(4096, 4096 * 2).unwrap()),
                Tag::new_free(Boundary::new(4096 * 3, 4096 * 18).unwrap()),
            ],
        );
    }

    #[test]
    fn test_arena_alloc_rounds_if_wrong_granule() {
        let mut arena = create_arena_with_static_tags(
            "test",
            Some(Boundary::new_unchecked(4096, 4096 * 20)),
            4096,
            None,
        );
        let a = arena.alloc_segment(1024);
        assert_eq!(a.unwrap().size, 4096);
    }

    #[test]
    fn test_arena_free() {
        let mut arena = create_arena_with_static_tags(
            "test",
            Some(Boundary::new_unchecked(4096, 4096 * 20)),
            4096,
            None,
        );
        assert_eq!(arena.tag_pool.len(), 100);

        // We need to test each case where we're freeing by scanning the tags linearly.
        // To do this we run through each case (comments from the `free` function)

        // Prev and next both non-free
        let a1 = arena.alloc(4096);
        let a2 = arena.alloc(4096);
        assert_eq!(arena.tag_pool.len(), 98);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096, 4096).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096 * 2, 4096).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096 * 3, 4096 * 18).unwrap()),
            ],
        );
        arena.free(a1);
        assert_eq!(arena.tag_pool.len(), 98);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096, 4096).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096 * 2, 4096).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096 * 3, 4096 * 18).unwrap()),
            ],
        );

        // Prev and next both free
        arena.free(a2);
        assert_eq!(arena.tag_pool.len(), 100);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096, 4096 * 20).unwrap()),
            ],
        );

        // Prev free, next non-free
        let a1 = arena.alloc(4096);
        let a2 = arena.alloc(4096);
        let a3 = arena.alloc(4096);
        arena.free(a1);
        assert_eq!(arena.tag_pool.len(), 97);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096, 4096).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096 * 2, 4096).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096 * 3, 4096).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096 * 4, 4096 * 17).unwrap()),
            ],
        );
        arena.free(a2);
        assert_eq!(arena.tag_pool.len(), 98);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096, 4096 * 2).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096 * 3, 4096).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096 * 4, 4096 * 17).unwrap()),
            ],
        );

        // Prev non-free, next free
        arena.free(a3);
        let a1 = arena.alloc(4096);
        assert_eq!(arena.tag_pool.len(), 99);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096, 4096).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096 * 2, 4096 * 19).unwrap()),
            ],
        );
        arena.free(a1);
        assert_eq!(arena.tag_pool.len(), 100);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096, 4096 * 20).unwrap()),
            ],
        );
    }

    // #[test]
    // fn test_arena_nesting() {
    //     // Create a page of tags we can share amongst the first arenas
    //     let mut page = Page4K([0; 4096]);
    //     const NUM_TAGS: usize = size_of::<Page4K>() / size_of::<TagItem>();
    //     let all_tags = unsafe { &mut *(&mut page as *mut Page4K as *mut [TagItem; NUM_TAGS]) };

    //     const NUM_ARENAS: usize = 4;
    //     const NUM_TAGS_PER_ARENA: usize = NUM_TAGS / NUM_ARENAS;
    //     let (arena1_tags, all_tags) = all_tags.split_at_mut(NUM_TAGS_PER_ARENA);
    //     let (arena2_tags, all_tags) = all_tags.split_at_mut(NUM_TAGS_PER_ARENA);
    //     let (arena3a_tags, all_tags) = all_tags.split_at_mut(NUM_TAGS_PER_ARENA);
    //     let (arena3b_tags, _) = all_tags.split_at_mut(NUM_TAGS_PER_ARENA);

    //     let mut arena1 = Arena::new_with_tags(
    //         "arena1",
    //         Some(Boundary::new_unchecked(4096, 4096 * 20)),
    //         4096,
    //         arena1_tags,
    //     );

    //     // Import all
    //     let mut arena2 = Arena::new_with_tags("arena2", None, 4096, arena2_tags);

    //     // Import first half
    //     let mut arena3a = Arena::new_with_tags(
    //         "arena3a",
    //         Some(Boundary::from(4096..4096 * 10)),
    //         4096,
    //         arena3a_tags,
    //     );

    //     // Import second half
    //     let mut arena3b = Arena::new_with_tags(
    //         "arena3b",
    //         Some(Boundary::from(4096 * 10..4096 * 21)),
    //         4096,
    //         arena3b_tags,
    //     );

    //     // Let's do some allocations
    // }
}
