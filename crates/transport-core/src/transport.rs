use anyhow::Result;
use async_trait::async_trait;

use crate::{TransportCapabilities, TransportKeyboard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFormat {
    Plain,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContent {
    pub text: String,
    pub format: TextFormat,
    pub keyboard: Option<TransportKeyboard>,
}

impl MessageContent {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            format: TextFormat::Plain,
            keyboard: None,
        }
    }

    pub fn html(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            format: TextFormat::Html,
            keyboard: None,
        }
    }

    pub fn with_keyboard(mut self, keyboard: TransportKeyboard) -> Self {
        self.keyboard = Some(keyboard);
        self
    }
}

#[async_trait]
pub trait BotTransport: Send + Sync + Clone + 'static {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities::default()
    }

    async fn send_text(&self, peer_id: i64, text: &str) -> Result<()>;

    async fn send_with_keyboard(
        &self,
        peer_id: i64,
        text: &str,
        keyboard: &TransportKeyboard,
    ) -> Result<()>;

    async fn send_message(&self, peer_id: i64, content: MessageContent) -> Result<()> {
        if let Some(keyboard) = content.keyboard {
            self.send_with_keyboard(peer_id, &content.text, &keyboard)
                .await
        } else {
            self.send_text(peer_id, &content.text).await
        }
    }

    async fn answer_callback(
        &self,
        event_id: &str,
        user_id: i64,
        peer_id: i64,
        text: Option<&str>,
    ) -> Result<()>;
}
