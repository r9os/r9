/// General page allocation errors.  Not specific to any particular implementation, and also includes higher-level errors.
#[derive(Debug, PartialEq)]
pub enum PageAllocError {
    OutOfBounds,
    MisalignedAddr,
    OutOfSpace,
    NotAllocated,
    UnableToMap,
}
