//! Transport-neutral bot abstractions.
//!
//! These types are intentionally independent from VK, Telegram, and concrete
//! SDKs. Legacy modules in the root crate re-export this crate during the
//! migration so existing code can keep using the old paths.

pub mod capabilities;
pub mod keyboard;
pub mod text_format;
pub mod transport;

pub use capabilities::TransportCapabilities;
pub use keyboard::{TransportButton, TransportKeyboard};
pub use text_format::strip_html;
pub use transport::{BotTransport, MessageContent, TextFormat};
