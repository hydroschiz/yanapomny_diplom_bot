//! Адаптеры для конкретных платформ.
//!
//! Содержит конвертацию `TransportKeyboard` в типы конкретных платформ.

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ReplyMarkup};

use crate::transport::traits::{BotTransport, TransportButton, TransportKeyboard};

/// Temporary Telegram transport adapter used while VK migration is phased in.
#[derive(Clone)]
pub struct TelegramTransport {
    bot: teloxide::Bot,
}

impl TelegramTransport {
    pub fn new(bot: teloxide::Bot) -> Self {
        Self { bot }
    }

    pub fn bot(&self) -> &teloxide::Bot {
        &self.bot
    }
}

#[async_trait::async_trait]
impl BotTransport for TelegramTransport {
    async fn send_text(&self, peer_id: i64, text: &str) -> anyhow::Result<()> {
        use teloxide::prelude::*;

        self.bot.send_message(ChatId(peer_id), text).await?;
        Ok(())
    }

    async fn send_with_keyboard(
        &self,
        peer_id: i64,
        text: &str,
        keyboard: &TransportKeyboard,
    ) -> anyhow::Result<()> {
        use teloxide::prelude::*;

        let markup = reply_markup_from_transport_keyboard(keyboard);
        self.bot
            .send_message(ChatId(peer_id), text)
            .reply_markup(markup)
            .await?;
        Ok(())
    }

    async fn answer_callback(
        &self,
        event_id: &str,
        _user_id: i64,
        _peer_id: i64,
        text: Option<&str>,
    ) -> anyhow::Result<()> {
        use teloxide::prelude::*;

        let request = self
            .bot
            .answer_callback_query(teloxide::types::CallbackQueryId(event_id.to_string()));

        match text {
            Some(text) => request.text(text).await?,
            None => request.await?,
        };

        Ok(())
    }
}

pub fn reply_markup_from_transport_keyboard(keyboard: &TransportKeyboard) -> ReplyMarkup {
    ReplyMarkup::InlineKeyboard(inline_keyboard_from_transport_keyboard(keyboard))
}

pub fn inline_keyboard_from_transport_keyboard(
    keyboard: &TransportKeyboard,
) -> InlineKeyboardMarkup {
    let rows: Vec<Vec<InlineKeyboardButton>> = keyboard
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|button| match button {
                    TransportButton::Callback { label, data } => {
                        InlineKeyboardButton::callback(label.clone(), data.clone())
                    }
                    TransportButton::Url { label, url } => {
                        let parsed_url = url::Url::parse(url)
                            .unwrap_or_else(|_| url::Url::parse("https://example.com").unwrap());
                        InlineKeyboardButton::url(label.clone(), parsed_url)
                    }
                })
                .collect()
        })
        .collect();

    InlineKeyboardMarkup::new(rows)
}
