#[cfg(platform = "nezha")]
pub mod nezha;
#[cfg(platform = "nezha")]
pub use crate::platform::nezha::*;

#[cfg(not(platform = "nezha"))]
pub mod virt;
#[cfg(not(platform = "nezha"))]
pub use crate::platform::virt::*;
