//! Transport-neutral presentation layer.
//!
//! This crate owns event normalization, command/callback parsing, keyboard
//! builders, and message rendering. It intentionally does not know about VK,
//! Telegram, MongoDB, or HTTP clients.

pub mod command;
pub mod event;
pub mod keyboard;
pub mod output;
pub mod payload;
pub mod renderer;
pub mod router;

pub use command::{parse_command, BotCommand, ParsedCommand};
pub use event::{IncomingCallback, IncomingEvent, IncomingGroupEvent, IncomingMessage};
pub use keyboard::KeyboardBuilder;
pub use output::{OutgoingCallbackAnswer, OutgoingMessage, RenderedResponse};
pub use payload::{parse_payload, CallbackPayload};
pub use renderer::{Notification, Renderer, TimezoneDisplay};
pub use router::{
    extract_group_mention_text, parse_channel_url, CallbackRoute, ChannelPlatform,
    ConversationState, MessageRoute, ParsedChannelLink, RouteContext, Router,
};
