pub mod allwinner;
pub mod virt;

#[cfg(feature = "virt")]
pub use crate::board::virt::*;

#[cfg(feature = "allwinner")]
pub use crate::board::allwinner::*;
