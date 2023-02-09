#[cfg(platform = "nezha")]
pub mod nezha;
#[cfg(platform = "nezha")]
pub use crate::platform::nezha::*;

#[cfg(not(platform))]
pub mod virt;
#[cfg(not(platform))]
pub use crate::platform::virt::*;
