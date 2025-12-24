//! Клавиатуры для подписок на каналы (Twitch/YouTube).

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

/// Клавиатура для списка подписок.
/// Кнопки: Удалить, Назад, Профиль.
pub fn channel_subs_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("🗑 Удалить", "sub_delete"),
            InlineKeyboardButton::callback("◀️ Назад", "back_main"),
        ],
        vec![
            InlineKeyboardButton::callback("👤 Профиль", "profile"),
        ],
    ])
}

/// Клавиатура для уведомлений о стримах/видео.
/// Кнопки: Профиль, Подписки.
pub fn stream_notification_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("👤 Профиль", "profile"),
        InlineKeyboardButton::callback("📺 Подписки", "subs"),
    ]])
}
