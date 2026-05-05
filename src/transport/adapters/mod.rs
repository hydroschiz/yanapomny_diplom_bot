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

        let markup: ReplyMarkup = keyboard.clone().into();
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

impl From<TransportKeyboard> for ReplyMarkup {
    fn from(tk: TransportKeyboard) -> Self {
        let markup: InlineKeyboardMarkup = tk.into();
        ReplyMarkup::InlineKeyboard(markup)
    }
}

impl From<TransportKeyboard> for InlineKeyboardMarkup {
    fn from(tk: TransportKeyboard) -> Self {
        let rows: Vec<Vec<InlineKeyboardButton>> = tk
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|btn| match btn {
                        TransportButton::Callback { label, data } => {
                            InlineKeyboardButton::callback(label, data)
                        }
                        TransportButton::Url { label, url } => {
                            let parsed_url = url::Url::parse(&url).unwrap_or_else(|_| {
                                // Fallback to a dummy URL if parsing fails
                                url::Url::parse("https://example.com").unwrap()
                            });
                            InlineKeyboardButton::url(label, parsed_url)
                        }
                    })
                    .collect()
            })
            .collect();

        InlineKeyboardMarkup::new(rows)
    }
}
