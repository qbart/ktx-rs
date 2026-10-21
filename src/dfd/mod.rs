//! Port of KTX-Software `external/dfdutils` (Data Format Descriptor utilities).

pub mod access;
pub mod createdfd;
pub mod khr_df;

pub use access::*;
pub use createdfd::*;

/// Port of `enum VkSuffix`: qualifier suffix to the format, in Vulkan terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VkSuffix {
    Unorm,
    Snorm,
    Uscaled,
    Sscaled,
    Uint,
    Sint,
    Sfloat,
    Ufloat,
    Srgb,
    Sfixed5,
}

/// Port of `enum VkCompScheme`: compression scheme, in Vulkan terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VkCompScheme {
    Bc1Rgb,
    Bc1Rgba,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc6h,
    Bc7,
    Etc2R8G8B8,
    Etc2R8G8B8A1,
    Etc2R8G8B8A8,
    EacR11,
    EacR11G11,
    Astc,
    Etc1s,
    Pvrtc,
    Pvrtc2,
}
