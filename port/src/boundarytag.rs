use core::{fmt, ptr::null_mut, slice};

use crate::mem::VirtRange;

#[cfg(not(test))]
use crate::println;

// TODO reserve recursive area in vmem

#[derive(Debug, PartialEq)]
pub enum BoundaryError {
    ZeroSize,
}

#[derive(Debug, PartialEq)]
pub enum AllocError {
    NoSpace,
}

#[cfg(test)]
type BoundaryResult<T> = core::result::Result<T, BoundaryError>;

#[derive(Copy, Clone, Debug, PartialEq)]
struct Boundary {
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

    fn end(&self) -> usize {
        self.start + self.size
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum TagType {
    Allocated,
    Free,
    Span,
}

#[derive(Copy, Clone, Debug, PartialEq)]
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
        Self { tag_type: TagType::Allocated, boundary }
    }

    fn new_free(boundary: Boundary) -> Self {
        Self { tag_type: TagType::Free, boundary }
    }

    fn new_span(boundary: Boundary) -> Self {
        Self { tag_type: TagType::Span, boundary }
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

    // #[cfg(test)]
    // fn new_free(boundary: Boundary) -> Self {
    //     Self { tag: Tag::new_free(boundary), next: null_mut(), prev: null_mut() }
    // }

    // #[cfg(test)]
    // fn new_span(boundary: Boundary) -> Self {
    //     Self { tag: Tag::new_span(boundary), next: null_mut(), prev: null_mut() }
    // }

    fn clear_links(&mut self) {
        self.next = null_mut();
        self.prev = null_mut();
    }
}

/// Stack of tags, useful for freelist
struct TagStack {
    tags: *mut TagItem,
}

impl TagStack {
    fn new() -> Self {
        Self { tags: null_mut() }
    }

    fn push(&mut self, tag: &mut TagItem) {
        if self.tags.is_null() {
            self.tags = tag;
        } else {
            tag.next = self.tags;
            unsafe { (*tag.next).prev = tag };
            self.tags = tag;
        }
    }

    fn pop(&mut self) -> *mut TagItem {
        if let Some(tag) = unsafe { self.tags.as_mut() } {
            self.tags = tag.next;
            if let Some(next_tag) = unsafe { self.tags.as_mut() } {
                next_tag.prev = null_mut();
            }
            tag.clear_links();
            tag as *mut TagItem
        } else {
            null_mut()
        }
    }

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
struct TagList {
    tags: *mut TagItem,
}

impl TagList {
    fn new() -> Self {
        Self { tags: null_mut() }
    }

    // ATM this is a simple linked list that assumes no overlaps.
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

    fn len(&self) -> usize {
        let mut n = 0;
        let mut curr_tag = self.tags;
        while let Some(tag) = unsafe { curr_tag.as_ref() } {
            n += 1;
            curr_tag = tag.next;
        }
        n
    }

    #[cfg(test)]
    fn tags_iter(&self) -> impl Iterator<Item = Tag> + '_ {
        let mut curr_tag_item = self.tags;
        core::iter::from_fn(move || {
            if let Some(item) = unsafe { curr_tag_item.as_ref() } {
                curr_tag_item = item.next;
                return Some(item.tag);
            } else {
                return None;
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

pub struct Arena {
    _name: &'static str,
    quantum: usize,
    used_tags: TagList,
    free_tags: TagStack,
    // TODO Add hashtable for allocated tags - makes it faster when freeing, given only an address
}

impl Arena {
    /// Only to be used for creation of initial heap
    pub fn new_with_static_range(
        name: &'static str,
        base: usize,
        size: usize,
        quantum: usize,
        static_range: VirtRange,
    ) -> Self {
        let tags_addr = unsafe { &mut *(static_range.start() as *mut TagItem) };
        let tags = unsafe { slice::from_raw_parts_mut(tags_addr, static_range.size()) };

        println!(
            "Arena::new_with_static_range name:{} base:{:x} size:{:x} quantum:{:x}",
            name, base, size, quantum
        );

        Self::new_with_tags(name, base, size, quantum, tags)
    }

    /// Create a new arena, assuming there is no dynamic allocation available,
    /// and all free tags come from the free_tags provided.
    fn new_with_tags(
        name: &'static str,
        base: usize,
        size: usize,
        quantum: usize,
        free_tags: &mut [TagItem],
    ) -> Self {
        assert_eq!(base % quantum, 0);
        assert_eq!(size % quantum, 0);
        assert!(base.checked_add(size).is_some());

        let mut arena = Self {
            _name: name,
            quantum: quantum,
            used_tags: TagList::new(),
            free_tags: TagStack::new(),
        };
        arena.add_free_tags(free_tags);
        arena.add_span(base, size);
        arena
    }

    pub fn add_span(&mut self, base: usize, size: usize) {
        self.used_tags.push({
            let item = unsafe { self.free_tags.pop().as_mut().expect("no free tags") };
            item.tag = Tag::new_span(Boundary::new_unchecked(base, size));
            item
        });
        self.used_tags.push({
            let item = unsafe { self.free_tags.pop().as_mut().expect("no free tags") };
            item.tag = Tag::new_free(Boundary::new_unchecked(base, size));
            item
        });
    }

    fn add_free_tags(&mut self, tags: &mut [TagItem]) {
        for tag in tags {
            tag.clear_links();
            self.free_tags.push(tag);
        }
    }

    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        let boundary = self.alloc_segment(size);
        if boundary.is_ok() {
            // TODO Register in allocation hashtable
            boundary.unwrap().start as *mut u8
        } else {
            null_mut()
        }
    }

    pub fn free(&mut self, addr: *mut u8) {
        // TODO Look up in allocation hashtable
        panic!("can't free before allocation hashtable set up");
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
        let mut curr_item = self.used_tags.tags;
        while let Some(item) = unsafe { curr_item.as_mut() } {
            if item.tag.tag_type == TagType::Free && item.tag.boundary.size >= size {
                // Mark this tag as allocated, and if there's any left over space, create and insert a new tag
                item.tag.tag_type = TagType::Allocated;
                if item.tag.boundary.size > size {
                    // Work out the size of the new free item, and change the size of the current, now allocated, item
                    let remainder = item.tag.boundary.size - size;
                    item.tag.boundary.size = size;

                    let new_item = unsafe { self.free_tags.pop().as_mut().expect("no free tags") };
                    new_item.tag = Tag::new_free(Boundary::new_unchecked(
                        item.tag.boundary.start + size,
                        remainder,
                    ));

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

    // Free addr.  We need to know size in case the previous allocation was
    // merged with others around it.
    // TODO Error on precondition fail
    // TODO Use hashmap if available
    // TODO Return Result
    fn free_segment(&mut self, boundary: Boundary) {
        // Need to manually scan the used tags
        let start = boundary.start as usize;
        let end = boundary.end();
        let mut curr_item = self.used_tags.tags;
        while let Some(item) = unsafe { curr_item.as_mut() } {
            if item.tag.boundary.start <= start && item.tag.boundary.end() <= end {
                break;
            }
            curr_item = item.next;
        }

        if curr_item.is_null() {
            // TODO Return error
            return;
        }

        // Found tag to free.  Cases:
        // Allocated segment encloses segment exactly:
        //  - If segment to the left is a span and segment to the right is a span or doesn't exist, then change segment to free
        //  - If segment to the left is free and segment to the right is a span or doesn't exist, then change left segment size
        // Allocated segment encloses segment to free with space to the left and right:
        //  - Split existing segment, create new tag for free segment, new tag for allocated segment to the right
        // Allocated segment starts before but ends at same point of segment to free:
        //  - If the segment to the right is a span, insert a new free segment.
        //  - If the segment to the right is free, merge by changing start of that segment.
        // Allocated segment starts at same point of segment to free but ends after;
        //  - If the segment to the left is a span, insert a new free segment.
        //  - If the segment to the left is free, merge by changing end of that segment.
    }

    #[cfg(test)]
    fn tags_iter(&self) -> impl Iterator<Item = Tag> + '_ {
        self.used_tags.tags_iter()
    }

    /// Checks that all invariants are correct.
    #[cfg(test)]
    fn assert_tags_are_consistent(&self) {
        // There must be at least 2 tags
        debug_assert!(self.used_tags.len() >= 2);

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
                TagType::Allocated => {
                    last_span_total += tag.boundary.size;
                    // First tag after span should have same start as span
                    if last_tag.is_some_and(|t| t.tag_type == TagType::Span) {
                        debug_assert_eq!(tag.boundary.start, last_tag.unwrap().boundary.start);
                    }
                }
                TagType::Free => {
                    last_span_total += tag.boundary.size;
                    // First tag after span should have same start as span
                    if last_tag.is_some_and(|t| t.tag_type == TagType::Span) {
                        debug_assert_eq!(tag.boundary.start, last_tag.unwrap().boundary.start);
                    }
                    // Free tag must be last in span
                    debug_assert_eq!(last_span_total, last_span.unwrap().boundary.size);
                }
            }
            last_tag = Some(tag);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

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
        let mut tag_stack = TagStack::new();

        assert_eq!(tag_stack.len(), 0);
        for tag in tags {
            tag_stack.push(tag);
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
        base: usize,
        size: usize,
        quantum: usize,
    ) -> Arena {
        let mut page = Page4K([0; 4096]);
        const NUM_TAGS: usize = size_of::<Page4K>() / size_of::<TagItem>();
        let tags = unsafe { &mut *(&mut page as *mut Page4K as *mut [TagItem; NUM_TAGS]) };
        Arena::new_with_tags(name, base, size, quantum, tags)
    }

    fn assert_tags_eq(arena: &Arena, expected: &[Tag]) {
        arena.assert_tags_are_consistent();
        let actual_tags = arena.tags_iter().collect::<Vec<Tag>>();
        assert_eq!(actual_tags, expected, "arena tag mismatch");
    }

    #[test]
    fn test_arena_create() {
        let arena = create_arena_with_static_tags("test", 4096, 4096 * 20, 4096);

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
        let mut arena = create_arena_with_static_tags("test", 4096, 4096 * 20, 4096);

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
        let mut arena = create_arena_with_static_tags("test", 4096, 4096 * 20, 4096);
        let a = arena.alloc_segment(1024);
        assert_eq!(a.unwrap().size, 4096);
    }

    #[test]
    fn test_arena_free() {
        let mut arena = create_arena_with_static_tags("test", 4096, 4096 * 20, 4096);

        // We need to test each case where we're freeing by scanning the tags linearly.
        // To do this we run through each case (comments from the `free` function)

        // Allocated segment encloses segment exactly:
        //  - If segment to the left is a span and segment to the right is a span or doesn't exist, then change segment to free
        let a = arena.alloc(4096);
        assert_tags_eq(
            &arena,
            &[
                Tag::new(TagType::Span, Boundary::new(4096, 4096 * 20).unwrap()),
                Tag::new(TagType::Allocated, Boundary::new(4096, 4096).unwrap()),
                Tag::new(TagType::Free, Boundary::new(4096 * 2, 4096 * 19).unwrap()),
            ],
        );
        arena.free(a);

        //  - If segment to the left is free and segment to the right is a span, or doesn't exist then change left segment size

        // Allocated segment encloses segment to free with space to the left and right:
        //  - Split existing segment, create new tag for free segment, new tag for allocated segment to the right

        // Allocated segment starts before but ends at same point of segment to free:
        //  - If the segment to the right is a span, insert a new free segment.
        //  - If the segment to the right is free, merge by changing start of that segment.
        // Allocated segment starts at same point of segment to free but ends after;
        //  - If the segment to the left is a span, insert a new free segment.
        //  - If the segment to the left is free, merge by changing end of that segment.
    }
}
