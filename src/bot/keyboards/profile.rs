//! Клавиатуры для профиля пользователя.

use teloxide::types::{InlineKeyboardButton as Btn, InlineKeyboardMarkup};

/// Главная клавиатура профиля.
///
/// Callback data:
/// - `profile_list` - список напоминаний
/// - `profile_setup` - настройки
/// - `profile_subs` - уведомления каналов
/// - `profile_referral` - реферальная программа (TODO)
/// - `profile_pay` - подписка
pub fn profile_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            Btn::callback("📋 Список напоминаний", "profile_list"),
            Btn::callback("⚙️ Настройки", "profile_setup"),
        ],
        vec![
            Btn::callback("📺 Уведомления", "profile_subs"),
            Btn::callback("👥 Рефералка", "profile_referral"),
        ],
        vec![
            Btn::callback("💎 Подписка", "profile_pay"),
        ],
    ])
}
