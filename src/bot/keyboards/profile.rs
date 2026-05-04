//! Клавиатуры для профиля пользователя.

use crate::transport::traits::{TransportButton, TransportKeyboard};

/// Главная клавиатура профиля.
///
/// Callback data:
/// - `profile_list` - список напоминаний
/// - `profile_setup` - настройки
/// - `profile_subs` - уведомления каналов
/// - `profile_referral` - реферальная программа (TODO)
/// - `profile_pay` - подписка
pub fn profile_keyboard() -> TransportKeyboard {
    TransportKeyboard::new(vec![
        vec![
            TransportButton::callback("📋 Список напоминаний", "profile_list"),
            TransportButton::callback("⚙️ Настройки", "profile_setup"),
        ],
        vec![
            TransportButton::callback("📺 Уведомления", "profile_subs"),
            TransportButton::callback("👥 Рефералка", "profile_referral"),
        ],
        vec![
            TransportButton::callback("💎 Подписка", "profile_pay"),
        ],
    ])
}
