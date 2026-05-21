//! VK transport adapter.
//!
//! Owns VK SDK details: keyboard conversion, event normalization, message send,
//! and callback answers. It intentionally has no application use-case knowledge.

pub mod event;
pub mod keyboard;
pub mod sender;

pub use event::{
    callback_payload, is_group_peer, normalize_callback, normalize_event, normalize_message,
    VkIncomingCallback, VkIncomingEvent, VkIncomingMessage,
};
pub use keyboard::{
    convert_keyboard, convert_keyboard_to_vk_api, sanitize_keyboard, vk_inline_capabilities,
    VkButton, VkKeyboard,
};
pub use sender::{random_id, VkTransport};
