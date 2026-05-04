//! Адаптеры для конкретных платформ.
//!
//! Содержит конвертацию `TransportKeyboard` в типы конкретных платформ.

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ReplyMarkup};

use crate::transport::traits::{TransportButton, TransportKeyboard};

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