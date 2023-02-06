pub mod allwinner;
pub mod virt;

#[cfg(board = "virt")]
pub use crate::board::virt::*;

#[cfg(board = "allwinner")]
pub use crate::board::allwinner::*;
