//! Клавиатуры для подписок на каналы (Twitch/YouTube).

use crate::transport::traits::{TransportButton, TransportKeyboard};

/// Клавиатура для списка подписок.
/// Кнопки: Удалить, Назад, Профиль.
pub fn channel_subs_keyboard() -> TransportKeyboard {
    TransportKeyboard::new(vec![
        vec![
            TransportButton::callback("🗑 Удалить", "sub_delete"),
            TransportButton::callback("◀️ Назад", "back_main"),
        ],
        vec![TransportButton::callback("👤 Профиль", "profile")],
    ])
}

/// Клавиатура для уведомлений о стримах/видео.
/// Кнопки: Профиль, Подписки.
pub fn stream_notification_keyboard() -> TransportKeyboard {
    TransportKeyboard::new(vec![vec![
        TransportButton::callback("👤 Профиль", "profile"),
        TransportButton::callback("📺 Подписки", "subs"),
    ]])
}
