#[cfg(platform = "nezha")]
pub mod nezha;
#[cfg(platform = "nezha")]
pub use crate::platform::nezha::*;

#[cfg(platform = "virt")]
pub mod virt;
#[cfg(platform = "virt")]
pub use crate::platform::virt::*;
