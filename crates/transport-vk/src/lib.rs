//! VK transport adapter.
//!
//! Phase 4 starts with VK keyboard normalization and capability-aware conversion.
//! The legacy root crate still owns the live `vk-bot-api` runtime adapter until
//! the composition root is moved to workspace crates.

pub mod keyboard;

pub use keyboard::{
    convert_keyboard, sanitize_keyboard, vk_inline_capabilities, VkButton, VkKeyboard,
};
