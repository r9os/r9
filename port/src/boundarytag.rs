use core::{ptr::null_mut, slice};

use crate::mem::VirtRange;

#[derive(Debug, PartialEq)]
pub enum BoundaryError {
    ZeroSize,
}

type BoundaryResult<T> = core::result::Result<T, BoundaryError>;

#[derive(Copy, Clone, Debug, PartialEq)]
struct Boundary {
    start: usize,
    size: usize,
}

impl Boundary {
    fn new(start: usize, size: usize) -> BoundaryResult<Self> {
        if size == 0 {
            Err(BoundaryError::ZeroSize)
        } else {
            Ok(Self { start, size })
        }
    }

    #[allow(dead_code)]
    fn overlaps(&self, other: &Boundary) -> bool {
        let boundary_end = self.start + self.size;
        let tag_end = other.start + other.size;
        (self.start <= other.start && boundary_end > other.start)
            || (self.start < tag_end && boundary_end >= tag_end)
            || (self.start <= other.start && boundary_end >= tag_end)
    }
}

struct Tag {
    boundary: Boundary,
    next: *mut Tag,
    prev: *mut Tag,
}

impl Tag {
    #[cfg(test)]
    fn new(boundary: Boundary) -> Self {
        Self { boundary, next: null_mut(), prev: null_mut() }
    }

    fn clear_links(&mut self) {
        self.next = null_mut();
        self.prev = null_mut();
    }
}

/// Stack of tags, useful for freelist
struct TagStack {
    tags: *mut Tag,
}

impl TagStack {
    fn new() -> Self {
        Self { tags: null_mut() }
    }

    fn push(&mut self, tag: &mut Tag) {
        if self.tags.is_null() {
            self.tags = tag;
        } else {
            tag.next = self.tags;
            unsafe { (*tag.next).prev = tag };
            self.tags = tag;
        }
    }

    fn pop(&mut self) -> *mut Tag {
        if let Some(tag) = unsafe { self.tags.as_mut() } {
            self.tags = tag.next;
            if let Some(next_tag) = unsafe { self.tags.as_mut() } {
                next_tag.prev = null_mut();
            }
            tag.clear_links();
            tag as *mut Tag
        } else {
            null_mut()
        }
    }

    #[cfg(test)]
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
    tags: *mut Tag,
}

impl TagList {
    fn new() -> Self {
        Self { tags: null_mut() }
    }

    // TODO add support for spans.  ATM this is a simple linked list that assumes no overlaps.
    fn push(&mut self, new_tag: &mut Tag) {
        if self.tags.is_null() {
            self.tags = new_tag;
        } else {
            let mut curr_tag = self.tags;
            while let Some(tag) = unsafe { curr_tag.as_mut() } {
                if tag.boundary.start > new_tag.boundary.start {
                    // Insert before tag
                    if let Some(prev_tag) = unsafe { tag.prev.as_mut() } {
                        prev_tag.next = new_tag;
                    } else {
                        // Inserting as first tag
                        self.tags = new_tag;
                    }
                    new_tag.next = tag;
                    tag.prev = new_tag;
                    return;
                }
                if tag.next.is_null() {
                    // Inserting as last tag
                    new_tag.prev = tag;
                    tag.next = new_tag;
                    return;
                }
                curr_tag = tag.next;
            }
        }
    }

    #[cfg(test)]
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
    fn boundaries_iter(&self) -> impl Iterator<Item = Boundary> + '_ {
        let mut curr_tag = self.tags;
        core::iter::from_fn(move || {
            if let Some(tag) = unsafe { curr_tag.as_ref() } {
                curr_tag = tag.next;
                return Some(tag.boundary);
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
    _quantum: usize,
    used_tags: TagList,
    free_tags: TagStack,
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
        let tags_addr = unsafe { &mut *(static_range.start() as *mut Tag) };
        let tags = unsafe { slice::from_raw_parts_mut(tags_addr, static_range.size()) };

        Self::new_with_tags(name, base, size, quantum, tags)
    }

    /// Create a new arena, assuming there is no dynamic allocation available,
    /// and all free tags come from the free_tags provided.
    fn new_with_tags(
        name: &'static str,
        base: usize,
        size: usize,
        quantum: usize,
        free_tags: &mut [Tag],
    ) -> Self {
        assert_eq!(base % quantum, 0);
        assert_eq!(size % quantum, 0);
        let end = base.checked_add(size).expect("Arena::new_with_tags base+end out of bounds");

        let mut arena = Self {
            _name: name,
            _quantum: quantum,
            used_tags: TagList::new(),
            free_tags: TagStack::new(),
        };
        arena.add_free_tags(free_tags);

        // Use tags to indicate the unusable boundaries
        if base > 0 {
            let boundary = Boundary::new(0, base).expect("invalid boundary");
            let tag = unsafe {
                arena.free_tags.pop().as_mut().expect("Arena::new_with_tags no free tags")
            };
            tag.boundary = boundary;
            arena.used_tags.push(tag);
        }
        if end <= usize::MAX {
            let boundary = Boundary::new(end, usize::MAX - end).expect("invalid boundary");
            let tag = unsafe {
                arena.free_tags.pop().as_mut().expect("Arena::new_with_tags no free tags")
            };
            tag.boundary = boundary;
            arena.used_tags.push(tag);
        }

        arena
    }

    fn add_free_tags(&mut self, tags: &mut [Tag]) {
        for tag in tags {
            tag.clear_links();
            self.free_tags.push(tag);
        }
    }

    #[cfg(test)]
    fn boundaries_iter(&self) -> impl Iterator<Item = Boundary> + '_ {
        self.used_tags.boundaries_iter()
    }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;

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
        assert_eq!(size_of::<Tag>(), 32);
        let mut page = Page4K([0; 4096]);
        const NUM_TAGS: usize = size_of::<Page4K>() / size_of::<Tag>();
        let tags = unsafe { &mut *(&mut page as *mut Page4K as *mut [Tag; NUM_TAGS]) };
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
        assert_eq!(list.boundaries_iter().collect::<Vec<Boundary>>(), []);

        let mut tag1 = Tag::new(Boundary::new(100, 100).unwrap());
        list.push(&mut tag1);
        assert_eq!(list.len(), 1);
        assert_eq!(
            list.boundaries_iter().collect::<Vec<Boundary>>(),
            [Boundary::new(100, 100).unwrap()]
        );

        // Insert new at end
        let mut tag2 = Tag::new(Boundary::new(500, 100).unwrap());
        list.push(&mut tag2);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list.boundaries_iter().collect::<Vec<Boundary>>(),
            [Boundary::new(100, 100).unwrap(), Boundary::new(500, 100).unwrap()]
        );

        // Insert new at start
        let mut tag3 = Tag::new(Boundary::new(0, 100).unwrap());
        list.push(&mut tag3);
        assert_eq!(list.len(), 3);
        assert_eq!(
            list.boundaries_iter().collect::<Vec<Boundary>>(),
            [
                Boundary::new(0, 100).unwrap(),
                Boundary::new(100, 100).unwrap(),
                Boundary::new(500, 100).unwrap()
            ]
        );

        // Insert new in middle
        let mut tag4 = Tag::new(Boundary::new(200, 100).unwrap());
        list.push(&mut tag4);
        assert_eq!(list.len(), 4);
        assert_eq!(
            list.boundaries_iter().collect::<Vec<Boundary>>(),
            [
                Boundary::new(0, 100).unwrap(),
                Boundary::new(100, 100).unwrap(),
                Boundary::new(200, 100).unwrap(),
                Boundary::new(500, 100).unwrap()
            ]
        );
    }

    #[test]
    fn test_arena() {
        assert_eq!(size_of::<Tag>(), 32);
        let mut page = Page4K([0; 4096]);
        const NUM_TAGS: usize = size_of::<Page4K>() / size_of::<Tag>();
        let tags = unsafe { &mut *(&mut page as *mut Page4K as *mut [Tag; NUM_TAGS]) };
        let arena = Arena::new_with_tags("test", 4096, 8192, 4096, tags);

        assert_eq!(
            arena.boundaries_iter().collect::<Vec<Boundary>>(),
            [Boundary::new(0, 4096).unwrap(), Boundary::new(12288, usize::MAX - 12288).unwrap()]
        );
    }
}
