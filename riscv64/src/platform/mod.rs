#[cfg(platform = "nezha")]
pub mod nezha;
#[cfg(platform = "nezha")]
pub use crate::platform::nezha::*;

#[cfg(not(platform = "virt"))]
pub mod virt;
#[cfg(not(platform = "virt"))]
pub use crate::platform::virt::*;
