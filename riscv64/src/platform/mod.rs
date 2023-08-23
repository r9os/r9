#[cfg(platform = "nezha")]
pub mod nezha;
#[cfg(platform = "nezha")]
pub use crate::platform::nezha::*;

#[cfg(any(test, platform = "virt", not(platform = "nezha")))]
pub mod virt;
#[cfg(any(test, platform = "virt", not(platform = "nezha")))]
pub use crate::platform::virt::*;
