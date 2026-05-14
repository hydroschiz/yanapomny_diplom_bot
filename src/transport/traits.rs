//! Compatibility re-export for legacy modules.
//!
//! The canonical transport-neutral contracts live in `transport-core`.

pub use transport_core::{
    BotTransport, MessageContent, TextFormat, TransportButton, TransportCapabilities,
    TransportKeyboard,
};
